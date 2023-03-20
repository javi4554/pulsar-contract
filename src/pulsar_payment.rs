#![no_std]
multiversx_sc::imports!();
multiversx_sc::derive_imports!();

mod cancellation;
mod schedule;
mod schedule_release;
mod schedule_type;

use cancellation::Cancellation;
use schedule::Schedule;
use schedule_type::ScheduleType;
use schedule_release::ScheduleRelease;

const ONE_SCHEDULE_TOKEN: u64 = 1_000u64;

#[multiversx_sc::derive::contract]
pub trait PulsarPayment {
    #[init]
    fn init(&self, schedule_token_id: TokenIdentifier, cancel_token_id: TokenIdentifier, fee: u64) {
        self.schedule_token_id().set(&schedule_token_id);
        self.cancel_token_id().set(&cancel_token_id);
        self.fee().set(fee);
    }

    #[endpoint(create)]
    #[payable("*")]
    fn create(
        &self,
        schedule_type: ScheduleType, // 0 vault; 1 pulsar pay; 2 vesting
        name: ManagedBuffer,
        cancelable: bool,
        receivers: ManagedVec<ManagedAddress>,
        releases: MultiValueEncoded<ScheduleRelease<Self::Api>>,
        #[payment_token] payment_token: EgldOrEsdtTokenIdentifier, 
        #[payment_nonce] payment_nonce: u64,
        #[payment_amount] payment_amount: BigUint,
    ) {
        let mut schedule_releases = ManagedVec::new();
        let mut total_amount = BigUint::zero();
        let mut total_amount_post_tax = BigUint::zero();

        require!(!releases.is_empty(), "Minimum 1 release!");
        require!(!receivers.is_empty(), "Minimum 1 receiver!");

        let start_date = releases.clone().into_iter().map(|release| release.start_date).min().unwrap();
        let end_date = releases.clone().into_iter().map(|release| release.end_date).max().unwrap();

        for release_request in releases {
            require!(release_request.end_date > release_request.start_date, "End date should not be at an earlier point than start date!");
            require!((1..=365 * 24 * 60 * 60).contains(&release_request.duration), "Not a valid interval duration!");
            require!(release_request.start_date >= self.blockchain().get_block_timestamp(), "Start date should not be in the past!");
            require!((release_request.end_date - release_request.start_date) % release_request.duration == 0, "Interval duration must be a multiple of the difference between end_date and start_date!");

            let amount_post_tax = release_request.amount.clone() * (BigUint::from(1000u64 - self.fee().get())) / BigUint::from(1000u64);
            let duration = BigUint::from(release_request.end_date - release_request.start_date);
            let amount_per_interval = amount_post_tax / duration.clone() / BigUint::from(receivers.len()) * release_request.duration;
            require!(amount_per_interval > 100000, "Minimum rate not reached. Please increase interval duration!");

            let amount_post_tax_calculated = amount_per_interval.clone() * duration * BigUint::from(receivers.len()) / release_request.duration;
            
            total_amount += release_request.amount.clone();
            total_amount_post_tax += amount_post_tax_calculated.clone();

            schedule_releases.push(ScheduleRelease {
                amount: amount_per_interval.clone(),
                start_date: release_request.start_date,
                end_date: release_request.end_date,
                duration: release_request.duration,
            });
        }

        require!(payment_amount == total_amount, "Total release amount does not match transaction amount!");

        let tax = payment_amount - total_amount_post_tax; 

        self.pay_egld_esdt(payment_token.clone(), payment_nonce, self.blockchain().get_owner_address(), tax);

        for receiver in &receivers {  
            let identifier = self.increment_last_id();
            let schedule = Schedule {
                schedule_type, 
                release_token: payment_token.clone(), 
                release_nonce: payment_nonce,
                start_date,
                end_date,
                name: name.clone(), 
                releases: schedule_releases.clone(),
                identifier, 
            };

            self.create_and_send(self.schedule_token_id().get(), BigUint::from(ONE_SCHEDULE_TOKEN), schedule, receiver);

            if cancelable {
                let cancellation = Cancellation { schedule_identifier: identifier, release_token: payment_token.clone(), release_nonce: payment_nonce, releases: schedule_releases.clone() };
                self.create_and_send(self.cancel_token_id().get(), BigUint::from(1u64), cancellation, self.blockchain().get_caller().clone());
            }
        }
    }

    fn increment_last_id(&self) -> u64 {
        let last_id = self.last_id().get();
        self.last_id().set(&last_id + 1);

        last_id + 1
    }
   
    #[endpoint(claim)]
    #[payable("*")]
    fn claim(&self) {
        let payments = self.call_value().all_esdt_transfers();
        self.verify_schedule_token_payments(&payments);

        for payment in &payments {
            self.claim_schedule(payment.token_identifier, payment.token_nonce, payment.amount);
        }
    }

    fn verify_schedule_token_payments(&self, payments: &ManagedVec<EsdtTokenPayment<Self::Api>>) {
        let schedule_token_id = self.schedule_token_id().get();

        for payment in payments {
            require!(payment.token_identifier == schedule_token_id, "Invalid schedule token!");
        }
    }
   
    fn claim_schedule(&self, payment_token: TokenIdentifier, payment_nonce: u64, payment_amount: BigUint) {
        let schedule = self.decode_token_attributes::<Schedule<Self::Api>>(&payment_token, payment_nonce);

        let mut releases = ManagedVec::new();

        for release in &schedule.releases {
            let claim_release_result = self.claim_release(release, payment_amount.clone(), schedule.identifier, schedule.release_token.clone(), schedule.release_nonce);

            match claim_release_result {
                OptionalValue::Some(release) => {
                    releases.push(release);
                },
                OptionalValue::None => {}
            };
        }

        self.send().esdt_local_burn(&payment_token, payment_nonce, &payment_amount);

        if !releases.is_empty() {
            let schedule_attributes = Schedule {
                schedule_type: schedule.schedule_type, 
                release_token: schedule.release_token,
                release_nonce: schedule.release_nonce,
                start_date: schedule.start_date,
                end_date: schedule.end_date,
                identifier: schedule.identifier, 
                name: schedule.name, 
                releases
            };
    
            self.create_and_send(self.schedule_token_id().get(), payment_amount, schedule_attributes, self.blockchain().get_caller());
        }
    }
   
    fn claim_release(
        &self, release: ScheduleRelease<Self::Api>, amount: BigUint, identifier: u64, schedule_token: EgldOrEsdtTokenIdentifier, schedule_nonce: u64,
    ) -> OptionalValue<ScheduleRelease<Self::Api>> {
        let current_date = self.blockchain().get_block_timestamp();
        let cancel_date = self.cancel_list(identifier).get();

        match self.get_claimable_amount_for_release(cancel_date, amount, release.clone()) {
            OptionalValue::Some(claimable_amount) => {
                if claimable_amount > 0 {
                    self.pay_egld_esdt(schedule_token, schedule_nonce, self.blockchain().get_caller(), claimable_amount);
                }
            },
            OptionalValue::None => {
                return OptionalValue::None;
            }
        };

        if cancel_date == 0 && current_date < release.end_date {
            let release_attributes = ScheduleRelease {
                amount: release.amount,
                start_date: current_date - ((current_date - release.start_date) % release.duration), 
                end_date: release.end_date, 
                duration: release.duration, 
            };

            return OptionalValue::Some(release_attributes);
        }

        OptionalValue::None
    }

    #[view(getClaimableAmount)]
    fn get_claimable_amount(
        &self,
        identifier: u64,
        amount: BigUint,
        releases: ManagedVec<ScheduleRelease<Self::Api>>
    ) -> BigUint {
        let mut claimable_amount = BigUint::zero();
        let cancel_date = self.cancel_list(identifier).get();

        for release in &releases {
            match self.get_claimable_amount_for_release(cancel_date, amount.clone(), release.clone()) {
                OptionalValue::Some(claimable_amount_for_release) => {
                    claimable_amount += claimable_amount_for_release;
                },
                OptionalValue::None => {}
            };
        }

        claimable_amount
    }

    #[view(isCancelled)]
    fn is_cancelled(&self, identifier: u64) -> bool {
        self.cancel_list(identifier).get() > 0
    }

    fn get_claimable_amount_for_release(
        &self,
        cancel_date: u64,
        amount: BigUint,
        release: ScheduleRelease<Self::Api>,
    ) -> OptionalValue<BigUint> {
        let current_date = self.blockchain().get_block_timestamp();

        if current_date <= release.start_date {
            // if cancelled and start is in the future, no need to include it in the release list
            if cancel_date > 0 {
                return OptionalValue::None;
            }

            return OptionalValue::Some(BigUint::zero());
        }

        let release_end: u64 = *[current_date, release.end_date, cancel_date].iter().filter(|v| *v > &0u64).min().unwrap();
        let claimable_intervals = (release_end - release.start_date) / release.duration;

        if claimable_intervals == 0 {
            return OptionalValue::Some(BigUint::zero());
        }

        let claimable_amount = amount * claimable_intervals * release.amount.clone() / BigUint::from(ONE_SCHEDULE_TOKEN);

        OptionalValue::Some(claimable_amount)
    }

    #[endpoint(cancel)]
    #[payable("*")]
    fn cancel(&self) {
        let payments = self.call_value().all_esdt_transfers();
        self.verify_cancellation_token_payments(&payments);

        for payment in &payments {
            self.cancel_internal(payment.token_identifier, payment.token_nonce, payment.amount);
        }
    }

    fn verify_cancellation_token_payments(&self, payments: &ManagedVec<EsdtTokenPayment<Self::Api>>) {
        let cancel_token_id = self.cancel_token_id().get();

        for payment in payments {
            require!(payment.token_identifier == cancel_token_id, "Invalid cancellation token!");
        }
    }

    fn cancel_internal(&self, payment_token: TokenIdentifier, payment_nonce: u64, payment_amount: BigUint) {
        let cancellation = self.decode_token_attributes::<Cancellation<Self::Api>>(&payment_token, payment_nonce);
        let current_date = self.blockchain().get_block_timestamp();

        self.cancel_list(cancellation.schedule_identifier).set(current_date); //vesting/payment

        for release in &cancellation.releases {
            if current_date < release.end_date { //token still active 
                let current_date_in_interval = current_date - ((current_date - release.start_date) % release.duration);
                let start_date = if current_date_in_interval > release.start_date {current_date_in_interval} else {release.start_date};
                let remaining_time = release.end_date - start_date;
                let remaining_amount = release.amount.clone() * BigUint::from(remaining_time) / BigUint::from(release.duration);
                self.pay_egld_esdt(cancellation.release_token.clone(), cancellation.release_nonce, self.blockchain().get_caller(), remaining_amount);
            }
        }

        self.send().esdt_local_burn(&payment_token, payment_nonce, &payment_amount); 
    }

    fn decode_token_attributes<T: TopDecode>( &self, token_id: &TokenIdentifier, token_nonce: u64) -> T {
        let token_info = self.blockchain().get_esdt_token_data( &self.blockchain().get_sc_address(), token_id, token_nonce);
        self.serializer().top_decode_from_managed_buffer::<T>(&token_info.attributes)
    }

    fn create_and_send<T: TopEncode>(&self, token_id: TokenIdentifier, amount: BigUint, attributes: T, receiver: ManagedAddress) -> u64 {
        let token_nonce = self.send().esdt_nft_create_compact(&token_id, &amount, &attributes);
        self.send().direct_esdt(&receiver, &token_id, token_nonce, &amount);
        token_nonce
    }

    fn pay_egld_esdt(&self, token: EgldOrEsdtTokenIdentifier, nonce: u64, receiver: ManagedAddress, amount: BigUint) {
        if token.is_egld() {
            self.send().direct_egld(&receiver, &amount);
        } else {
            self.send().direct_esdt(&receiver, &token.unwrap_esdt(), nonce, &amount);
        }
    }

    #[view(getScheduleTokenId)]
    #[storage_mapper("schedule_token_id")]
    fn schedule_token_id(&self) -> SingleValueMapper<TokenIdentifier>; 

    #[view(getCancelTokenId)]
    #[storage_mapper("cancel_token_id")]
    fn cancel_token_id(&self) -> SingleValueMapper<TokenIdentifier>; 
    
    #[storage_mapper("cancel_list")]
    fn cancel_list(&self, nonce: u64) -> SingleValueMapper<u64>;

    #[storage_mapper("last_id")]
    fn last_id(&self) -> SingleValueMapper<u64>;

    #[view(getFee)]
    #[storage_mapper("fee")]
    fn fee(&self) -> SingleValueMapper<u64>;  // value of x represents a fee of 0.x%

}