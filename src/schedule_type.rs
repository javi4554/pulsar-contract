multiversx_sc::derive_imports!();
multiversx_sc::imports!();

#[derive(NestedEncode, NestedDecode, TopEncode, TopDecode, TypeAbi, PartialEq, Eq, Clone, Copy)]
pub enum ScheduleType {
  Vault,
  PulsarPayment,
  Vesting,
}