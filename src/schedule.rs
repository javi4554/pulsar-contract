use crate::{schedule_type::ScheduleType, schedule_release::ScheduleRelease};

multiversx_sc::derive_imports!();
multiversx_sc::imports!();

#[derive(NestedEncode, NestedDecode, TypeAbi, TopEncode, TopDecode, Clone)]
pub struct Schedule<M: ManagedTypeApi> {
  pub schedule_type: ScheduleType, // 0 vault; 1 pulsar pay; 2 vesting
  pub identifier: u64,
  pub name: ManagedBuffer<M>,
  pub start_date: u64,
  pub end_date: u64,
  pub release_token: EgldOrEsdtTokenIdentifier<M>,
  pub release_nonce: u64,
  pub releases: ManagedVec<M, ScheduleRelease<M>>,
}