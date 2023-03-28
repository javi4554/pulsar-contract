use crate::{payment_type::PaymentType, release::Release};

multiversx_sc::derive_imports!();
multiversx_sc::imports!();

#[derive(NestedEncode, NestedDecode, TypeAbi, TopEncode, TopDecode, Clone)]
pub struct Payment<M: ManagedTypeApi> {
  pub version: u8,
  pub payment_type: PaymentType, // 0 vault; 1 pulsar pay; 2 vesting
  pub identifier: u64,
  pub name: ManagedBuffer<M>,
  pub start_date: u64,
  pub end_date: u64,
  pub release_token: EgldOrEsdtTokenIdentifier<M>,
  pub release_nonce: u64,
  pub creator: ManagedAddress<M>,
  pub amount: BigUint<M>,
  pub cancelable: bool,
  pub releases: ManagedVec<M, Release<M>>,
}