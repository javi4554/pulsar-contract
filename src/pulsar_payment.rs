#![no_std]
multiversx_sc::imports!();
multiversx_sc::derive_imports!();

mod cancellation;
mod payment;
mod release;
mod payment_type;

use cancellation::Cancellation;
use payment::Payment;
use payment_type::PaymentType;
use release::Release;

const ONE_PAYMENT_TOKEN: u64 = 1_000u64;

#[multiversx_sc::derive::contract]
pub trait PulsarPayment {
    #[init]
    fn init(&self, payment_token_id: TokenIdentifier, cancel_token_id: TokenIdentifier, fee: u64) {
        self.payment_token_id().set(&payment_token_id);
        self.cancel_token_id().set(&cancel_token_id);
        self.fee().set(fee);
    }

    #[endpoint(setFee)]
    #[only_owner]
    fn set_fee(&self, fee: u64) {
        self.fee().set(fee);
    }

    #[endpoint(create)]
    #[payable("*")]
    fn create(
        &self,
        payment_type: PaymentType, // 0 vault; 1 pulsar pay; 2 vesting
        name: ManagedBuffer,
        cancelable: bool,
        receivers: ManagedVec<ManagedAddress>,
        releases: MultiValueEncoded<Release<Self::Api>>,
        #[payment_token] token: EgldOrEsdtTokenIdentifier, 
        #[payment_nonce] nonce: u64,
        #[payment_amount] amount: BigUint,
    ) {
        let mut payment_releases = ManagedVec::new();
        let mut total_amount = BigUint::zero();
        let mut total_amount_post_tax = BigUint::zero();

        require!(!releases.is_empty(), "Minimum 1 release!");
        require!(!receivers.is_empty(), "Minimum 1 receiver!");

        let start_date = releases.clone().into_iter().map(|release| release.start_date).min().unwrap();
        let end_date = releases.clone().into_iter().map(|release| release.end_date).max().unwrap();

        for release_request in releases {
            require!(release_request.end_date > release_request.start_date, "End date should not be at an earlier point than start date!");
            require!((1..=365 * 24 * 60 * 60).contains(&release_request.interval_seconds), "Not a valid interval duration!");
            require!(release_request.start_date >= self.blockchain().get_block_timestamp(), "Start date should not be in the past!");
            require!((release_request.end_date - release_request.start_date) % release_request.interval_seconds == 0, "Interval duration must be a multiple of the difference between end_date and start_date!");

            let amount_post_tax = release_request.amount.clone() * (BigUint::from(1000u64 - self.fee().get())) / BigUint::from(1000u64);
            let interval_seconds = BigUint::from(release_request.end_date - release_request.start_date);
            let amount_per_interval = amount_post_tax / interval_seconds.clone() / BigUint::from(receivers.len()) * release_request.interval_seconds;

            if amount != 1u64 { // not is nft
                require!(amount_per_interval > 100_000, "Minimum rate not reached. Please increase interval duration!");
            }

            let amount_post_tax_calculated = amount_per_interval.clone() * interval_seconds * BigUint::from(receivers.len()) / release_request.interval_seconds;
            
            total_amount += release_request.amount.clone();
            total_amount_post_tax += amount_post_tax_calculated.clone();

            payment_releases.push(Release {
                amount: amount_per_interval.clone(),
                start_date: release_request.start_date,
                end_date: release_request.end_date,
                interval_seconds: release_request.interval_seconds,
            });
        }

        require!(amount == total_amount, "Total release amount does not match transaction amount!");

        let tax = amount - total_amount_post_tax.clone(); 

        self.pay_egld_esdt(token.clone(), nonce, self.blockchain().get_owner_address(), tax);

        for receiver in &receivers {  
            let identifier = self.increment_last_id();
            let payment = Payment {
                version: 2u8,
                payment_type, 
                identifier, 
                name: name.clone(), 
                start_date,
                end_date,
                release_token: token.clone(), 
                release_nonce: nonce,
                creator: self.blockchain().get_caller().clone(),
                amount: total_amount_post_tax.clone(),
                cancelable,
                releases: payment_releases.clone(),
            };

            self.create_and_send(self.payment_token_id().get(), BigUint::from(ONE_PAYMENT_TOKEN), payment, receiver);

            if cancelable {
                let cancellation = Cancellation { payment_identifier: identifier, release_token: token.clone(), release_nonce: nonce, releases: payment_releases.clone() };
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
        self.verify_payment_token_payments(&payments);

        for payment in &payments {
            self.claim_payment(payment.token_identifier, payment.token_nonce, payment.amount);
        }
    }

    fn verify_payment_token_payments(&self, payments: &ManagedVec<EsdtTokenPayment<Self::Api>>) {
        let payment_token_id = self.payment_token_id().get();

        for payment in payments {
            require!(payment.token_identifier == payment_token_id, "Invalid payment token!");
        }
    }
   
    fn claim_payment(&self, token: TokenIdentifier, nonce: u64, amount: BigUint) {
        let payment = self.decode_token_attributes::<Payment<Self::Api>>(&token, nonce);

        let mut releases = ManagedVec::new();

        for release in &payment.releases {
            let claim_release_result = self.claim_release(release, amount.clone(), payment.identifier, payment.release_token.clone(), payment.release_nonce);

            match claim_release_result {
                OptionalValue::Some(release) => {
                    releases.push(release);
                },
                OptionalValue::None => {}
            };
        }

        self.send().esdt_local_burn(&token, nonce, &amount);

        if !releases.is_empty() {
            let payment_attributes = Payment {
                version: payment.version,
                payment_type: payment.payment_type, 
                identifier: payment.identifier, 
                name: payment.name, 
                start_date: payment.start_date,
                end_date: payment.end_date,
                release_token: payment.release_token,
                release_nonce: payment.release_nonce,
                creator: payment.creator,
                amount: payment.amount,
                cancelable: payment.cancelable,
                releases
            };
    
            self.create_and_send(self.payment_token_id().get(), amount, payment_attributes, self.blockchain().get_caller());
        }
    }
   
    fn claim_release(
        &self,
        release: Release<Self::Api>,
        amount: BigUint,
        identifier: u64,
        payment_token: EgldOrEsdtTokenIdentifier,
        payment_nonce: u64,
    ) -> OptionalValue<Release<Self::Api>> {
        let current_date = self.blockchain().get_block_timestamp();
        let cancel_date = self.cancel_list(identifier).get();

        if cancel_date > 0 && cancel_date <= release.start_date {
            return OptionalValue::None;
        }

        if current_date <= release.start_date {
            return OptionalValue::Some(release);
        }

        let release_end: u64 = *[current_date, release.end_date, cancel_date].iter().filter(|v| *v > &0u64).min().unwrap();
        let claimable_intervals = (release_end - release.start_date) / release.interval_seconds;

        if claimable_intervals == 0 {
            return OptionalValue::Some(release);
        }

        let claimable_amount = amount * claimable_intervals * release.amount.clone() / BigUint::from(ONE_PAYMENT_TOKEN);

        self.pay_egld_esdt(payment_token, payment_nonce, self.blockchain().get_caller(), claimable_amount);
    
        if cancel_date == 0 && current_date < release.end_date {
            let release_attributes = Release {
                amount: release.amount,
                start_date: current_date - ((current_date - release.start_date) % release.interval_seconds), 
                end_date: release.end_date, 
                interval_seconds: release.interval_seconds, 
            };

            return OptionalValue::Some(release_attributes);
        }

        OptionalValue::None
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

    fn cancel_internal(&self, token: TokenIdentifier, nonce: u64, amount: BigUint) {
        let cancellation = self.decode_token_attributes::<Cancellation<Self::Api>>(&token, nonce);
        let current_date = self.blockchain().get_block_timestamp();

        self.cancel_list(cancellation.payment_identifier).set(current_date); //vesting/payment

        for release in &cancellation.releases {
            if current_date < release.end_date { //token still active 
                let current_date_in_interval = current_date - ((current_date - release.start_date) % release.interval_seconds);
                let start_date = if current_date_in_interval > release.start_date {current_date_in_interval} else {release.start_date};
                let remaining_time = release.end_date - start_date;
                let remaining_amount = release.amount.clone() * BigUint::from(remaining_time) / BigUint::from(release.interval_seconds);
                self.pay_egld_esdt(cancellation.release_token.clone(), cancellation.release_nonce, self.blockchain().get_caller(), remaining_amount);
            }
        }

        self.send().esdt_local_burn(&token, nonce, &amount); 
    }

    fn decode_token_attributes<T: TopDecode>( &self, token_id: &TokenIdentifier, token_nonce: u64) -> T {
        let token_info = self.blockchain().get_esdt_token_data( &self.blockchain().get_sc_address(), token_id, token_nonce);
        self.serializer().top_decode_from_managed_buffer::<T>(&token_info.attributes)
    }

    fn create_and_send<T: TopEncode>(&self,
        token_id: TokenIdentifier,
        amount: BigUint,
        attributes: T,
        receiver: ManagedAddress
    ) -> u64 {
        let token_nonce = self.send().esdt_nft_create_compact(&token_id, &amount, &attributes);
        self.send().direct_esdt(&receiver, &token_id, token_nonce, &amount);
        token_nonce
    }

    fn pay_egld_esdt(&self, token: EgldOrEsdtTokenIdentifier, nonce: u64, receiver: ManagedAddress, amount: BigUint) {
        if amount == BigUint::from(0u32) {
            return;
        }
        
        if token.is_egld() {
            self.send().direct_egld(&receiver, &amount);
        } else {
            self.send().direct_esdt(&receiver, &token.unwrap_esdt(), nonce, &amount);
        }
    }

    #[view(getPaymentTokenId)]
    #[storage_mapper("payment_token_id")]
    fn payment_token_id(&self) -> SingleValueMapper<TokenIdentifier>; 

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