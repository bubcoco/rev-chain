use crate::precompiles;
use alloy_consensus::transaction::Transaction;
use alloy_primitives::{Address, TxKind, U256, address, keccak256};
use evm2::{
    Evm, EvmFeatures, EvmTypesHost, ExecutionConfig, Precompiles, SpecId, Version,
    ethereum::{
        TxEnvelope, default_settle_gas, eip1559, eip2930, eip4844, eip7702, finalize_gas, legacy,
    },
    evm::handler::{GasSettlement, TxHandlerHooks},
    precompiles::{Precompile, PrecompileId},
    registry::{HandlerError, HandlerResult, TxRegistry, handler},
};
use reth_ethereum::evm::EvmFactory;

const GAS_FEE_GRANT: Address = address!("0x0000000000000000000000000000000000001006");
const REVENUE_RATIO: Address = address!("0x0000000000000000000000000000000000001004");
const TREASURY_REGISTRY: Address = address!("0x0000000000000000000000000000000000001005");
const FEE_GRANT_FLAG_STORAGE: U256 = U256::from_be_bytes([
    0x33, 0x0b, 0xb6, 0x44, 0x90, 0x68, 0xd1, 0x7e, 0x38, 0x15, 0xa0, 0x45, 0x68, 0x5a, 0x05, 0xa1,
    0x06, 0x74, 0x1a, 0x6e, 0x96, 0x09, 0x86, 0xb3, 0xc7, 0x2e, 0xb8, 0x6c, 0xb6, 0x92, 0xda, 0x00,
]);
const fn small_u256(value: u8) -> U256 {
    let mut bytes = [0; 32];
    bytes[31] = value;
    U256::from_be_bytes(bytes)
}

const MINIMUM_GRANTER_BALANCE: U256 = U256::from_be_bytes([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0d, 0xe0, 0xb6, 0xb3,
    0xa7, 0x64, 0x00, 0x00,
]);
const GAS_FEE_GRANT_STORAGE_SLOT: U256 = small_u256(2);
const GRANTS_COUNTER_SLOT: U256 = small_u256(3);
const PERIODIC_ALLOWANCE: U256 = small_u256(2);
const DATA_GAS_PER_BLOB: u64 = 131_072;

#[derive(Debug, Default)]
pub(crate) struct FeeGrantRuntimeState {
    settlement: Option<FeeGrantSettlement>,
}

#[derive(Debug)]
struct FeeGrantSettlement {
    granter: Address,
    root_slot: U256,
    upfront_fee: U256,
    spend_limit: U256,
    period_limit: U256,
    period_can_spend: U256,
    period_reset: U256,
    latest_transaction: U256,
    period: U256,
    periodic: bool,
    wildcard: bool,
}

pub(crate) struct RustEvmTypes;

impl EvmTypesHost for RustEvmTypes {
    type ConfigSelector = evm2::evm::config::BaseEvmConfigSelector;
    type SpecId = SpecId;
    type Tx = TxEnvelope;
    type EvmExt = FeeGrantRuntimeState;
    type MessageExt = ();
    type MessageResultExt = ();
    type TxEnvExt = ();
    type TxResultExt = ();
    type BlockEnvExt = ();
    type Host<'a> = Evm<'a, Self>;
}

#[derive(Debug, Default, Clone)]
pub(crate) struct RustEvmFactory;

impl EvmFactory for RustEvmFactory {
    type Types = RustEvmTypes;
    type SpecId = SpecId;

    fn spec_id(&self, spec: SpecId) -> SpecId {
        spec
    }

    fn execution_config(
        &self,
        spec: SpecId,
        mut version: Version,
    ) -> ExecutionConfig<RustEvmTypes> {
        version
            .features
            .remove(EvmFeatures::BALANCE_CHECK | EvmFeatures::BALANCE_TOP_UP);
        ExecutionConfig::for_spec_and_version(spec, version)
    }

    fn tx_registry(&self, spec: SpecId) -> TxRegistry<RustEvmTypes, evm2::TxResult<RustEvmTypes>> {
        let mut registry = TxRegistry::new().with_handler(
            0,
            TxEnvelope::as_legacy,
            handler(
                legacy::prepare_with_hooks::<RustEvmTypes, FeeGrantHooks>,
                legacy::execute_prepared::<RustEvmTypes, FeeGrantHooks>,
            ),
        );

        if spec.enables(SpecId::BERLIN) {
            registry.register(
                1,
                TxEnvelope::as_eip2930,
                handler(
                    eip2930::prepare_with_hooks::<RustEvmTypes, FeeGrantHooks>,
                    eip2930::execute_prepared::<RustEvmTypes, FeeGrantHooks>,
                ),
            );
        }
        if spec.enables(SpecId::LONDON) {
            registry.register(
                2,
                TxEnvelope::as_eip1559,
                handler(
                    eip1559::prepare_with_hooks::<RustEvmTypes, FeeGrantHooks>,
                    eip1559::execute_prepared::<RustEvmTypes, FeeGrantHooks>,
                ),
            );
        }
        if spec.enables(SpecId::CANCUN) {
            registry.register(
                3,
                TxEnvelope::as_eip4844,
                handler(
                    eip4844::prepare_with_hooks::<RustEvmTypes, FeeGrantHooks>,
                    eip4844::execute_prepared::<RustEvmTypes, FeeGrantHooks>,
                ),
            );
        }
        if spec.enables(SpecId::PRAGUE) {
            registry.register(
                4,
                TxEnvelope::as_eip7702,
                handler(
                    eip7702::prepare_with_hooks::<RustEvmTypes, FeeGrantHooks>,
                    eip7702::execute_prepared::<RustEvmTypes, FeeGrantHooks>,
                ),
            );
        }

        registry
    }

    fn precompiles(&self, spec: SpecId) -> Precompiles<RustEvmTypes> {
        let mut precompiles = Precompiles::base(spec);
        precompiles.as_map_mut().extend([
            Precompile::new(
                address!("0x0000000000000000000000000000000000001001"),
                PrecompileId::custom("native-minter"),
                precompiles::native_minter,
            ),
            Precompile::new(
                address!("0x0000000000000000000000000000000000001002"),
                PrecompileId::custom("address-registry"),
                precompiles::address_registry,
            ),
            Precompile::new(
                address!("0x0000000000000000000000000000000000001003"),
                PrecompileId::custom("gas-price"),
                precompiles::gas_price,
            ),
            Precompile::new(
                address!("0x0000000000000000000000000000000000001004"),
                PrecompileId::custom("revenue-ratio"),
                precompiles::revenue_ratio,
            ),
            Precompile::new(
                address!("0x0000000000000000000000000000000000001005"),
                PrecompileId::custom("treasury-registry"),
                precompiles::treasury_registry,
            ),
            Precompile::new(
                address!("0x0000000000000000000000000000000000001006"),
                PrecompileId::custom("gas-fee-grant"),
                precompiles::gas_fee_grant,
            ),
        ]);
        precompiles
    }
}

struct FeeGrantHooks;

impl TxHandlerHooks<RustEvmTypes> for FeeGrantHooks {
    fn adjust_intrinsic_gas(
        host: &mut Evm<'_, RustEvmTypes>,
        _envelope: &TxEnvelope,
        _intrinsic: &mut u64,
        _initial_state_gas: &mut u64,
        _floor_gas: &mut u64,
    ) -> HandlerResult<()> {
        host.ext_mut().settlement = None;
        Ok(())
    }

    fn before_execution(
        host: &mut Evm<'_, RustEvmTypes>,
        envelope: &TxEnvelope,
        caller: Address,
        upfront_fee: U256,
    ) -> HandlerResult<()> {
        if !host.feature(EvmFeatures::FEE_CHARGE) {
            return Ok(());
        }

        let sender_balance = host.state_mut().account(&caller)?.balance();
        let value = envelope.value();
        let is_candidate = read_storage(host, caller, FEE_GRANT_FLAG_STORAGE)? != U256::ZERO;
        if !is_candidate {
            let maximum_cost = max_transaction_cost(envelope).saturating_add(value);
            if sender_balance < maximum_cost {
                return Err(HandlerError::InsufficientFunds {
                    fee: maximum_cost,
                    balance: sender_balance,
                });
            }
            return evm2::ethereum::charge_upfront(host, caller, upfront_fee);
        }

        if sender_balance < value {
            return Err(HandlerError::InsufficientFunds {
                fee: value,
                balance: sender_balance,
            });
        }

        let Some(settlement) = resolve_grant(host, caller, envelope.kind(), upfront_fee)? else {
            let required = upfront_fee.saturating_add(value);
            if sender_balance < required {
                return Err(HandlerError::InsufficientFunds {
                    fee: required,
                    balance: sender_balance,
                });
            }
            return evm2::ethereum::charge_upfront(host, caller, upfront_fee);
        };

        if settlement.upfront_fee > host.state_mut().account(&settlement.granter)?.balance()
            || settlement.upfront_fee > settlement.spend_limit && !settlement.spend_limit.is_zero()
            || settlement.periodic && settlement.upfront_fee > settlement.period_can_spend
        {
            let required = upfront_fee.saturating_add(value);
            if sender_balance < required {
                return Err(HandlerError::InsufficientFunds {
                    fee: required,
                    balance: sender_balance,
                });
            }
            return evm2::ethereum::charge_upfront(host, caller, upfront_fee);
        }

        let granter_balance = host.state_mut().account(&settlement.granter)?.balance();
        if granter_balance < MINIMUM_GRANTER_BALANCE {
            lazy_revoke(host, caller, settlement.root_slot)?;
            let required = upfront_fee.saturating_add(value);
            if sender_balance < required {
                return Err(HandlerError::InsufficientFunds {
                    fee: required,
                    balance: sender_balance,
                });
            }
            return evm2::ethereum::charge_upfront(host, caller, upfront_fee);
        }

        let mut granter = host.state_mut().account(&settlement.granter)?;
        granter.set_balance(granter_balance - settlement.upfront_fee);
        drop(granter);
        host.ext_mut().settlement = Some(settlement);
        Ok(())
    }

    fn settle_transaction(
        host: &mut Evm<'_, RustEvmTypes>,
        envelope: &TxEnvelope,
        gas: GasSettlement<RustEvmTypes>,
    ) -> HandlerResult<evm2::TxResult<RustEvmTypes>> {
        let Some(settlement) = host.ext_mut().settlement.take() else {
            return default_settle_gas(host, gas);
        };

        let gas_price = gas.gas_price;
        let gas_limit = gas.gas_limit;
        let block_number = host.block().number;
        let result = finalize_gas(host, gas)?;
        if host.feature(EvmFeatures::FEE_CHARGE) {
            let gas_used = result.tx_gas_used();
            let gas_remaining = gas_limit.saturating_sub(gas_used);
            let refund = U256::from(gas_remaining) * gas_price;
            host.state_mut()
                .account(&settlement.granter)?
                .add_balance(refund);

            let beneficiary_gas_price = if host.spec_id().enables(SpecId::LONDON) {
                gas_price.saturating_sub(host.block().basefee)
            } else {
                gas_price
            };
            let coinbase_delta = U256::from(gas_used) * beneficiary_gas_price;
            if !coinbase_delta.is_zero() {
                distribute_revenue(host, envelope.kind(), settlement.granter, coinbase_delta)?;
            }
            update_grant_budget(
                host,
                &settlement,
                block_number,
                settlement.upfront_fee.saturating_sub(refund),
            )?;
        }
        Ok(result)
    }
}

fn resolve_grant(
    host: &mut Evm<'_, RustEvmTypes>,
    grantee: Address,
    destination: TxKind,
    upfront_fee: U256,
) -> HandlerResult<Option<FeeGrantSettlement>> {
    let program = match destination {
        TxKind::Call(address) => address,
        TxKind::Create => Address::ZERO,
    };
    let scoped_root = grant_root(grantee, program);
    let wildcard_root = grant_root(grantee, Address::ZERO);
    let wildcard_allowance = read_storage(host, GAS_FEE_GRANT, add_slot(wildcard_root, 1))?;
    let scoped_allowance = read_storage(host, GAS_FEE_GRANT, add_slot(scoped_root, 1))?;
    if wildcard_allowance.is_zero() && scoped_allowance.is_zero() {
        return Ok(None);
    }

    let (root_slot, allowance, wildcard) = if !wildcard_allowance.is_zero() {
        (wildcard_root, wildcard_allowance, true)
    } else {
        (scoped_root, scoped_allowance, false)
    };
    let block_number = host.block().number;
    let expiry = read_storage(host, GAS_FEE_GRANT, add_slot(root_slot, 6))?;
    if !expiry.is_zero() && block_number > expiry {
        return Ok(None);
    }

    let granter_word = read_storage(host, GAS_FEE_GRANT, root_slot)?;
    let granter = address_from_word(granter_word);
    if granter.is_zero() {
        return Ok(None);
    }

    let spend_limit = read_storage(host, GAS_FEE_GRANT, add_slot(root_slot, 2))?;
    let period_limit = read_storage(host, GAS_FEE_GRANT, add_slot(root_slot, 3))?;
    let period_can_spend = read_storage(host, GAS_FEE_GRANT, add_slot(root_slot, 4))?;
    let period_reset = read_storage(host, GAS_FEE_GRANT, add_slot(root_slot, 5))?;
    let latest_transaction = read_storage(host, GAS_FEE_GRANT, add_slot(root_slot, 7))?;
    let period = read_storage(host, GAS_FEE_GRANT, add_slot(root_slot, 8))?;
    let periodic = allowance == PERIODIC_ALLOWANCE;
    let period_can_spend = if latest_transaction.saturating_add(period) < period_reset {
        period_limit
    } else {
        period_can_spend
    };
    let granter_balance = host.state_mut().account(&granter)?.balance();
    if granter_balance < MINIMUM_GRANTER_BALANCE {
        lazy_revoke(host, grantee, root_slot)?;
        return Ok(None);
    }
    if upfront_fee > granter_balance
        || !spend_limit.is_zero() && upfront_fee > spend_limit
        || periodic && upfront_fee > period_can_spend
    {
        return Ok(None);
    }

    Ok(Some(FeeGrantSettlement {
        granter,
        root_slot,
        upfront_fee,
        spend_limit,
        period_limit,
        period_can_spend,
        period_reset,
        latest_transaction,
        period,
        periodic,
        wildcard,
    }))
}

fn update_grant_budget(
    host: &mut Evm<'_, RustEvmTypes>,
    settlement: &FeeGrantSettlement,
    block_number: U256,
    actual_cost: U256,
) -> HandlerResult<()> {
    if settlement.wildcard {
        return Ok(());
    }
    if !settlement.spend_limit.is_zero() {
        write_storage(
            host,
            GAS_FEE_GRANT,
            add_slot(settlement.root_slot, 2),
            settlement.spend_limit.saturating_sub(actual_cost),
        )?;
    }
    if settlement.periodic {
        let source_budget = if settlement
            .latest_transaction
            .saturating_add(settlement.period)
            < settlement.period_reset
        {
            settlement.period_limit
        } else {
            settlement.period_can_spend
        };
        write_storage(
            host,
            GAS_FEE_GRANT,
            add_slot(settlement.root_slot, 4),
            source_budget.saturating_sub(actual_cost),
        )?;
        write_storage(
            host,
            GAS_FEE_GRANT,
            add_slot(settlement.root_slot, 7),
            block_number,
        )?;
    }
    Ok(())
}

fn distribute_revenue(
    host: &mut Evm<'_, RustEvmTypes>,
    destination: TxKind,
    sender_share_beneficiary: Address,
    coinbase_delta: U256,
) -> HandlerResult<()> {
    let TxKind::Call(destination) = destination else {
        let beneficiary = host.block().beneficiary;
        host.state_mut()
            .account(&beneficiary)?
            .add_balance(coinbase_delta);
        return Ok(());
    };
    if read_storage(host, REVENUE_RATIO, U256::from(2))? != U256::ONE
        || read_storage(
            host,
            REVENUE_RATIO,
            U256::from_be_slice(keccak256(destination.as_slice()).as_slice()),
        )? != U256::ONE
    {
        let beneficiary = host.block().beneficiary;
        host.state_mut()
            .account(&beneficiary)?
            .add_balance(coinbase_delta);
        return Ok(());
    }

    let sender_share =
        coinbase_delta * read_storage(host, REVENUE_RATIO, U256::from(3))? / U256::from(100);
    let coinbase_share =
        coinbase_delta * read_storage(host, REVENUE_RATIO, U256::from(4))? / U256::from(100);
    let provider_share =
        coinbase_delta * read_storage(host, REVENUE_RATIO, U256::from(5))? / U256::from(100);
    let treasury_share =
        coinbase_delta * read_storage(host, REVENUE_RATIO, U256::from(6))? / U256::from(100);
    let remainder = coinbase_delta
        .saturating_sub(sender_share)
        .saturating_sub(coinbase_share)
        .saturating_sub(provider_share)
        .saturating_sub(treasury_share);
    host.state_mut()
        .account(&sender_share_beneficiary)?
        .add_balance(sender_share);

    let treasury = address_from_word(read_storage(host, TREASURY_REGISTRY, U256::from(2))?);
    let provider = address_from_word(read_storage(host, TREASURY_REGISTRY, U256::from(3))?);
    let mut coinbase_remainder = coinbase_share + remainder;
    if treasury.is_zero() {
        coinbase_remainder += treasury_share;
    } else {
        host.state_mut()
            .account(&treasury)?
            .add_balance(treasury_share);
    }
    if provider.is_zero() {
        coinbase_remainder += provider_share;
    } else {
        host.state_mut()
            .account(&provider)?
            .add_balance(provider_share);
    }
    let beneficiary = host.block().beneficiary;
    host.state_mut()
        .account(&beneficiary)?
        .add_balance(coinbase_remainder);
    Ok(())
}

fn lazy_revoke(
    host: &mut Evm<'_, RustEvmTypes>,
    grantee: Address,
    root_slot: U256,
) -> HandlerResult<()> {
    write_storage(host, grantee, FEE_GRANT_FLAG_STORAGE, U256::ZERO)?;
    for offset in 0..=8 {
        write_storage(host, GAS_FEE_GRANT, add_slot(root_slot, offset), U256::ZERO)?;
    }
    let counter_slot = mapping_slot(GRANTS_COUNTER_SLOT, grantee);
    let counter = read_storage(host, GAS_FEE_GRANT, counter_slot)?;
    if !counter.is_zero() {
        write_storage(host, GAS_FEE_GRANT, counter_slot, counter - U256::ONE)?;
    }
    Ok(())
}

fn max_transaction_cost(envelope: &TxEnvelope) -> U256 {
    let gas_cost = U256::from(envelope.gas_limit()) * U256::from(envelope.max_fee_per_gas());
    let blob_cost = envelope
        .max_fee_per_blob_gas()
        .zip(envelope.blob_versioned_hashes())
        .map(|(max_fee, hashes)| {
            U256::from(DATA_GAS_PER_BLOB) * U256::from(hashes.len()) * U256::from(max_fee)
        })
        .unwrap_or_default();
    gas_cost.saturating_add(blob_cost)
}

fn read_storage(
    host: &mut Evm<'_, RustEvmTypes>,
    address: Address,
    slot: U256,
) -> HandlerResult<U256> {
    Ok(host.state_mut().storage_slot(&address, slot)?.current())
}

fn write_storage(
    host: &mut Evm<'_, RustEvmTypes>,
    address: Address,
    slot: U256,
    value: U256,
) -> HandlerResult<()> {
    host.state_mut().storage_slot(&address, slot)?.write(value);
    Ok(())
}

fn grant_root(grantee: Address, program: Address) -> U256 {
    let grantee_root = keccak256(mapping_key(GAS_FEE_GRANT_STORAGE_SLOT, grantee));
    U256::from_be_slice(keccak256(mapping_key_bytes(grantee_root.as_slice(), program)).as_slice())
}

fn mapping_slot(slot: U256, key: Address) -> U256 {
    U256::from_be_slice(keccak256(mapping_key(slot, key)).as_slice())
}

fn mapping_key(slot: U256, key: Address) -> [u8; 64] {
    let mut data = [0_u8; 64];
    data[12..32].copy_from_slice(key.as_slice());
    data[32..].copy_from_slice(&slot.to_be_bytes::<32>());
    data
}

fn mapping_key_bytes(root: &[u8], key: Address) -> [u8; 64] {
    let mut data = [0_u8; 64];
    data[..32].copy_from_slice(root);
    data[44..].copy_from_slice(key.as_slice());
    data
}

fn add_slot(slot: U256, offset: u64) -> U256 {
    slot.wrapping_add(U256::from(offset))
}

fn address_from_word(word: U256) -> Address {
    Address::from_slice(&word.to_be_bytes::<32>()[12..])
}
