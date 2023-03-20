multiversx_sc::derive_imports!();
multiversx_sc::imports!();

#[derive(NestedEncode, NestedDecode, TypeAbi, TopEncode, TopDecode, ManagedVecItem, Clone)]
pub struct ScheduleRelease<M: ManagedTypeApi> {
  pub start_date: u64,
  pub end_date: u64,
  pub duration: u64,
  pub amount: BigUint<M>,
}