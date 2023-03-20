use crate::{schedule_release::ScheduleRelease};

multiversx_sc::derive_imports!();
multiversx_sc::imports!();

#[derive(NestedEncode, NestedDecode, TypeAbi, TopEncode, TopDecode)]
pub struct Cancellation<M: ManagedTypeApi> {
  pub schedule_identifier: u64,
  pub release_token: EgldOrEsdtTokenIdentifier<M>,
  pub release_nonce: u64,
  pub releases: ManagedVec<M, ScheduleRelease<M>>,
}