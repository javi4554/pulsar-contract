multiversx_sc::derive_imports!();
multiversx_sc::imports!();

#[derive(NestedEncode, NestedDecode, TypeAbi, TopEncode, TopDecode, ManagedVecItem, Clone)]
pub struct Release<M: ManagedTypeApi> {
  pub start_date: u64,
  pub end_date: u64,
  pub interval_seconds: u64,
  pub amount: BigUint<M>,
}