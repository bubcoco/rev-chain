use crate::evm::RustEvmTypes as BaseEvmTypes;
use alloy_primitives::{Address, Bytes, U256, keccak256};
use evm2::{
    Evm,
    evm::precompile::PrecompileOutput,
    interpreter::{GasTracker, Message},
    precompiles::{PrecompileError, PrecompileHalt, PrecompileResult},
};

const NATIVE_MINTER: Address =
    alloy_primitives::address!("0x0000000000000000000000000000000000001001");
const ADDRESS_REGISTRY: Address =
    alloy_primitives::address!("0x0000000000000000000000000000000000001002");
const GAS_PRICE: Address = alloy_primitives::address!("0x0000000000000000000000000000000000001003");
const REVENUE_RATIO: Address =
    alloy_primitives::address!("0x0000000000000000000000000000000000001004");
const TREASURY_REGISTRY: Address =
    alloy_primitives::address!("0x0000000000000000000000000000000000001005");
const GAS_FEE_GRANT: Address =
    alloy_primitives::address!("0x0000000000000000000000000000000000001006");
const FEE_GRANT_FLAG_STORAGE: U256 = U256::from_be_bytes([
    0x33, 0x0b, 0xb6, 0x44, 0x90, 0x68, 0xd1, 0x7e, 0x38, 0x15, 0xa0, 0x45, 0x68, 0x5a, 0x05, 0xa1,
    0x06, 0x74, 0x1a, 0x6e, 0x96, 0x09, 0x86, 0xb3, 0xc7, 0x2e, 0xb8, 0x6c, 0xb6, 0x92, 0xda, 0x00,
]);

const FALSE: U256 = U256::ZERO;
const TRUE: U256 = U256::ONE;

pub(crate) fn native_minter(
    evm: &mut Evm<'_, BaseEvmTypes>,
    message: &Message<BaseEvmTypes>,
    gas: &mut GasTracker,
) -> PrecompileResult {
    let input = evm.message_input(message).to_vec();
    let cost = if input.len() < 4 {
        2_000
    } else {
        fixed_cost(
            &input,
            &["owner()", "initialized()", "totalsupply()"],
            1_000,
            2_000,
        )
    };
    gas.try_spend(cost)?;
    require_direct_call(message, NATIVE_MINTER)?;
    let (selector, calldata) = split_input(&input)?;
    ensure_contract(evm, NATIVE_MINTER)?;

    match selector {
        s if s == selector_of("owner()") => word_output(storage_at(evm, NATIVE_MINTER, U256::ONE)?),
        s if s == selector_of("initialized()") => {
            word_output(storage_at(evm, NATIVE_MINTER, U256::ZERO)?)
        }
        s if s == selector_of("totalsupply()") => {
            word_output(storage_at(evm, NATIVE_MINTER, U256::from(2))?)
        }
        s if s == selector_of("initializeOwner(address)") && !message.is_static() => {
            let Some(owner) = calldata_word(calldata, 0) else {
                return bool_output(false);
            };
            if owner.is_zero() || storage_at(evm, NATIVE_MINTER, U256::ZERO)? == TRUE {
                return bool_output(false);
            }
            store(evm, NATIVE_MINTER, U256::ONE, owner)?;
            store(evm, NATIVE_MINTER, U256::ZERO, TRUE)?;
            bool_output(true)
        }
        s if s == selector_of("transferOwnership(address)") && !message.is_static() => {
            let Some(owner) = calldata_word(calldata, 0) else {
                return bool_output(false);
            };
            if !is_owner(evm, NATIVE_MINTER, message.caller)? || address_from_word(owner).is_zero()
            {
                return bool_output(false);
            }
            store(evm, NATIVE_MINTER, U256::ONE, owner)?;
            bool_output(true)
        }
        s if s == selector_of("mint(address,uint256)") && !message.is_static() => {
            let (Some(recipient), Some(amount)) =
                (calldata_address(calldata, 0), calldata_word(calldata, 32))
            else {
                return bool_output(false);
            };
            if !is_owner(evm, NATIVE_MINTER, message.caller)?
                || recipient.is_zero()
                || amount.is_zero()
            {
                return bool_output(false);
            }
            let balance = evm
                .state_mut()
                .account(&recipient)?
                .balance()
                .checked_add(amount);
            let supply = storage_at(evm, NATIVE_MINTER, U256::from(2))?.checked_add(amount);
            let (Some(balance), Some(supply)) = (balance, supply) else {
                return bool_output(false);
            };
            evm.state_mut().account(&recipient)?.set_balance(balance);
            store(evm, NATIVE_MINTER, U256::from(2), supply)?;
            bool_output(true)
        }
        _ => Err(rejected()),
    }
}

pub(crate) fn address_registry(
    evm: &mut Evm<'_, BaseEvmTypes>,
    message: &Message<BaseEvmTypes>,
    gas: &mut GasTracker,
) -> PrecompileResult {
    let input = evm.message_input(message).to_vec();
    gas.try_spend(fixed_cost(
        &input,
        &[
            "owner()",
            "initialized()",
            "discovery(address)",
            "contains(address)",
        ],
        1_000,
        2_000,
    ))?;
    require_direct_call(message, ADDRESS_REGISTRY)?;
    let (selector, calldata) = split_input(&input)?;
    ensure_contract(evm, ADDRESS_REGISTRY)?;

    match selector {
        s if s == selector_of("owner()") => {
            word_output(storage_at(evm, ADDRESS_REGISTRY, U256::ONE)?)
        }
        s if s == selector_of("initialized()") => {
            word_output(storage_at(evm, ADDRESS_REGISTRY, U256::ZERO)?)
        }
        s if s == selector_of("contains(address)") => {
            let Some(key) = calldata_address(calldata, 0) else {
                return bool_output(false);
            };
            bool_output(
                !storage_at(evm, ADDRESS_REGISTRY, mapping_slot(U256::from(2), key)?)?.is_zero(),
            )
        }
        s if s == selector_of("discovery(address)") => {
            let Some(key) = calldata_address(calldata, 0) else {
                return bool_output(false);
            };
            word_output(storage_at(
                evm,
                ADDRESS_REGISTRY,
                mapping_slot(U256::from(2), key)?,
            )?)
        }
        s if s == selector_of("initializeOwner(address)") && !message.is_static() => {
            let Some(owner) = calldata_word(calldata, 0) else {
                return bool_output(false);
            };
            if owner.is_zero() || storage_at(evm, ADDRESS_REGISTRY, U256::ZERO)? == TRUE {
                return bool_output(false);
            }
            store(evm, ADDRESS_REGISTRY, U256::ONE, owner)?;
            store(evm, ADDRESS_REGISTRY, U256::ZERO, TRUE)?;
            bool_output(true)
        }
        s if s == selector_of("transferOwnership(address)") && !message.is_static() => {
            let Some(owner) = calldata_word(calldata, 0) else {
                return bool_output(false);
            };
            if !is_owner(evm, ADDRESS_REGISTRY, message.caller)?
                || address_from_word(owner).is_zero()
            {
                return bool_output(false);
            }
            store(evm, ADDRESS_REGISTRY, U256::ONE, owner)?;
            bool_output(true)
        }
        s if s == selector_of("addToRegistry(address,address)") && !message.is_static() => {
            let (Some(target), Some(initiator), Some(value)) = (
                calldata_address(calldata, 0),
                calldata_address(calldata, 32),
                calldata_word(calldata, 32),
            ) else {
                return bool_output(false);
            };
            if !is_owner(evm, ADDRESS_REGISTRY, message.caller)?
                || target.is_zero()
                || initiator.is_zero()
            {
                return bool_output(false);
            }
            store(
                evm,
                ADDRESS_REGISTRY,
                mapping_slot(U256::from(2), target)?,
                value,
            )?;
            bool_output(true)
        }
        s if s == selector_of("removeFromRegistry(address)") && !message.is_static() => {
            let Some(target) = calldata_address(calldata, 0) else {
                return bool_output(false);
            };
            let slot = mapping_slot(U256::from(2), target)?;
            if !is_owner(evm, ADDRESS_REGISTRY, message.caller)?
                || storage_at(evm, ADDRESS_REGISTRY, slot)?.is_zero()
            {
                return bool_output(false);
            }
            store(evm, ADDRESS_REGISTRY, slot, U256::ZERO)?;
            bool_output(true)
        }
        _ => Err(rejected()),
    }
}

pub(crate) fn gas_price(
    evm: &mut Evm<'_, BaseEvmTypes>,
    message: &Message<BaseEvmTypes>,
    gas: &mut GasTracker,
) -> PrecompileResult {
    let input = evm.message_input(message).to_vec();
    gas.try_spend(fixed_cost(
        &input,
        &["owner()", "initialized()", "gasPrice()", "status()"],
        1_000,
        2_000,
    ))?;
    require_direct_call(message, GAS_PRICE)?;
    let (selector, calldata) = split_input(&input)?;
    ensure_contract(evm, GAS_PRICE)?;

    match selector {
        s if s == selector_of("owner()") => word_output(storage_at(evm, GAS_PRICE, U256::ONE)?),
        s if s == selector_of("initialized()") => {
            word_output(storage_at(evm, GAS_PRICE, U256::ZERO)?)
        }
        s if s == selector_of("status()") => {
            word_output(storage_at(evm, GAS_PRICE, U256::from(2))?)
        }
        s if s == selector_of("gasPrice()") => {
            word_output(storage_at(evm, GAS_PRICE, U256::from(3))?)
        }
        s if s == selector_of("initializeOwner(address)") && !message.is_static() => {
            initialize_owner(evm, GAS_PRICE, calldata)
        }
        s if s == selector_of("transferOwnership(address)") && !message.is_static() => {
            transfer_ownership(evm, GAS_PRICE, message.caller, calldata)
        }
        s if s == selector_of("enable()") && !message.is_static() => {
            set_owner_slot(evm, GAS_PRICE, message.caller, U256::from(2), TRUE)
        }
        s if s == selector_of("disable()") && !message.is_static() => {
            set_owner_slot(evm, GAS_PRICE, message.caller, U256::from(2), U256::ZERO)
        }
        s if s == selector_of("setGasPrice(uint256)") && !message.is_static() => {
            let Some(value) = calldata_word(calldata, 0) else {
                return bool_output(false);
            };
            set_owner_slot(evm, GAS_PRICE, message.caller, U256::from(3), value)
        }
        _ => Err(rejected()),
    }
}

pub(crate) fn revenue_ratio(
    evm: &mut Evm<'_, BaseEvmTypes>,
    message: &Message<BaseEvmTypes>,
    gas: &mut GasTracker,
) -> PrecompileResult {
    let input = evm.message_input(message).to_vec();
    gas.try_spend(fixed_cost(
        &input,
        &[
            "owner()",
            "initialized()",
            "senderRatio()",
            "coinbaseRatio()",
            "providerRatio()",
            "treasuryRatio()",
            "isRegistered(address)",
        ],
        1_000,
        2_000,
    ))?;
    require_direct_call(message, REVENUE_RATIO)?;
    let (selector, calldata) = split_input(&input)?;
    let exists = evm
        .state_mut()
        .account_info_untracked(&REVENUE_RATIO)?
        .is_some();

    match selector {
        s if s == selector_of("owner()") => optional_word(evm, exists, REVENUE_RATIO, U256::ONE),
        s if s == selector_of("initialized()") => {
            optional_word(evm, exists, REVENUE_RATIO, U256::ZERO)
        }
        s if s == selector_of("status()") => {
            optional_word(evm, exists, REVENUE_RATIO, U256::from(2))
        }
        s if s == selector_of("senderRatio()") => {
            optional_word(evm, exists, REVENUE_RATIO, U256::from(3))
        }
        s if s == selector_of("coinbaseRatio()") => {
            optional_word(evm, exists, REVENUE_RATIO, U256::from(4))
        }
        s if s == selector_of("providerRatio()") => {
            optional_word(evm, exists, REVENUE_RATIO, U256::from(5))
        }
        s if s == selector_of("treasuryRatio()") => {
            optional_word(evm, exists, REVENUE_RATIO, U256::from(6))
        }
        s if s == selector_of("isRegistered(address)") => {
            let Some(target) = calldata_address(calldata, 0) else {
                return bool_output(false);
            };
            if !exists {
                return bool_output(false);
            }
            bool_output(storage_at(evm, REVENUE_RATIO, registered_slot(target))? == TRUE)
        }
        s if s == selector_of("initializeOwner(address)") && !message.is_static() => {
            if !can_initialize_owner(evm, exists, REVENUE_RATIO, calldata)? {
                return bool_output(false);
            }
            ensure_contract(evm, REVENUE_RATIO)?;
            initialize_owner(evm, REVENUE_RATIO, calldata)
        }
        s if s == selector_of("transferOwnership(address)") && !message.is_static() => {
            if !exists {
                return bool_output(false);
            }
            transfer_ownership(evm, REVENUE_RATIO, message.caller, calldata)
        }
        s if s == selector_of("enable()") && !message.is_static() => {
            set_existing_owner_slot(evm, exists, message.caller, U256::from(2), TRUE)
        }
        s if s == selector_of("disable()") && !message.is_static() => {
            set_existing_owner_slot(evm, exists, message.caller, U256::from(2), U256::ZERO)
        }
        s if s == selector_of("setRevenueRatio(uint8,uint8,uint8,uint8)")
            && !message.is_static() =>
        {
            set_revenue_ratio(evm, exists, message.caller, calldata)
        }
        s if s == selector_of("registerContract(address)") && !message.is_static() => {
            update_registration(evm, exists, message.caller, calldata, TRUE)
        }
        s if s == selector_of("unregisterContract(address)") && !message.is_static() => {
            update_registration(evm, exists, message.caller, calldata, U256::ZERO)
        }
        _ => Err(rejected()),
    }
}

pub(crate) fn treasury_registry(
    evm: &mut Evm<'_, BaseEvmTypes>,
    message: &Message<BaseEvmTypes>,
    gas: &mut GasTracker,
) -> PrecompileResult {
    let input = evm.message_input(message).to_vec();
    gas.try_spend(fixed_cost(
        &input,
        &["owner()", "initialized()", "treasuryAt()", "providerAt()"],
        1_000,
        2_000,
    ))?;
    require_direct_call(message, TREASURY_REGISTRY)?;
    let (selector, calldata) = split_input(&input)?;
    ensure_contract(evm, TREASURY_REGISTRY)?;

    match selector {
        s if s == selector_of("owner()") => {
            word_output(storage_at(evm, TREASURY_REGISTRY, U256::ONE)?)
        }
        s if s == selector_of("initialized()") => {
            word_output(storage_at(evm, TREASURY_REGISTRY, U256::ZERO)?)
        }
        s if s == selector_of("treasuryAt()") => {
            word_output(storage_at(evm, TREASURY_REGISTRY, U256::from(2))?)
        }
        s if s == selector_of("providerAt()") => {
            word_output(storage_at(evm, TREASURY_REGISTRY, U256::from(3))?)
        }
        s if s == selector_of("initializeOwner(address)") && !message.is_static() => {
            initialize_owner(evm, TREASURY_REGISTRY, calldata)
        }
        s if s == selector_of("transferOwnership(address)") && !message.is_static() => {
            transfer_ownership(evm, TREASURY_REGISTRY, message.caller, calldata)
        }
        s if s == selector_of("setTreasury(address)") && !message.is_static() => {
            set_nonzero_owner_slot(
                evm,
                TREASURY_REGISTRY,
                message.caller,
                U256::from(2),
                calldata,
            )
        }
        s if s == selector_of("setProvider(address)") && !message.is_static() => {
            set_nonzero_owner_slot(
                evm,
                TREASURY_REGISTRY,
                message.caller,
                U256::from(3),
                calldata,
            )
        }
        _ => Err(rejected()),
    }
}

pub(crate) fn gas_fee_grant(
    evm: &mut Evm<'_, BaseEvmTypes>,
    message: &Message<BaseEvmTypes>,
    gas: &mut GasTracker,
) -> PrecompileResult {
    let input = evm.message_input(message).to_vec();
    gas.try_spend(fixed_cost(
        &input,
        &[
            "initializeOwner(address)",
            "transferOwnership(address)",
            "setFeeGrant(address,address,address,uint256,uint32,uint256,uint256)",
            "revokeFeeGrant(address,address)",
            "wildcard(address)",
        ],
        2_000,
        1_000,
    ))?;
    require_direct_call(message, GAS_FEE_GRANT)?;
    let (selector, calldata) = split_input(&input)?;
    ensure_contract(evm, GAS_FEE_GRANT)?;
    let block_number = evm.block().number;

    match selector {
        s if s == selector_of("owner()") => word_output(storage_at(evm, GAS_FEE_GRANT, U256::ONE)?),
        s if s == selector_of("initialized()") => {
            word_output(storage_at(evm, GAS_FEE_GRANT, U256::ZERO)?)
        }
        s if s == selector_of("initializeOwner(address)") && !message.is_static() => {
            initialize_owner(evm, GAS_FEE_GRANT, calldata)
        }
        s if s == selector_of("transferOwnership(address)") && !message.is_static() => {
            transfer_ownership(evm, GAS_FEE_GRANT, message.caller, calldata)
        }
        s if s == selector_of("grant(address,address)") => grant_view(evm, calldata, block_number),
        s if s == selector_of("periodCanSpend(address,address)") => {
            period_can_spend(evm, calldata, block_number)
        }
        s if s == selector_of("periodReset(address,address)") => {
            period_reset(evm, calldata, block_number)
        }
        s if s == selector_of("isExpired(address,address)") => {
            is_expired(evm, calldata, block_number)
        }
        s if s == selector_of("isGrantedForProgram(address,address)") => {
            grant_exists(evm, calldata, false)
        }
        s if s == selector_of("isGrantedForAllProgram(address)") => {
            grant_exists(evm, calldata, true)
        }
        s if s
            == selector_of(
                "setFeeGrant(address,address,address,uint256,uint32,uint256,uint256)",
            )
            && !message.is_static() =>
        {
            set_fee_grant(evm, message.caller, calldata, block_number)
        }
        s if s == selector_of("revokeFeeGrant(address,address)") && !message.is_static() => {
            revoke_fee_grant(evm, message.caller, calldata)
        }
        s if s == selector_of("wildcard(address)") && !message.is_static() => {
            set_wildcard(evm, message.caller, calldata)
        }
        _ => Err(rejected()),
    }
}

fn initialize_owner(
    evm: &mut Evm<'_, BaseEvmTypes>,
    contract: Address,
    calldata: &[u8],
) -> PrecompileResult {
    let Some(owner) = calldata_word(calldata, 0) else {
        return bool_output(false);
    };
    if owner.is_zero() || storage_at(evm, contract, U256::ZERO)? == TRUE {
        return bool_output(false);
    }
    store(evm, contract, U256::ONE, owner)?;
    store(evm, contract, U256::ZERO, TRUE)?;
    bool_output(true)
}

fn transfer_ownership(
    evm: &mut Evm<'_, BaseEvmTypes>,
    contract: Address,
    caller: Address,
    calldata: &[u8],
) -> PrecompileResult {
    let Some(owner) = calldata_word(calldata, 0) else {
        return bool_output(false);
    };
    if !is_owner(evm, contract, caller)? || address_from_word(owner).is_zero() {
        return bool_output(false);
    }
    store(evm, contract, U256::ONE, owner)?;
    bool_output(true)
}

fn set_owner_slot(
    evm: &mut Evm<'_, BaseEvmTypes>,
    contract: Address,
    caller: Address,
    slot: U256,
    value: U256,
) -> PrecompileResult {
    if !is_owner(evm, contract, caller)? {
        return bool_output(false);
    }
    store(evm, contract, slot, value)?;
    bool_output(true)
}

fn set_existing_owner_slot(
    evm: &mut Evm<'_, BaseEvmTypes>,
    exists: bool,
    caller: Address,
    slot: U256,
    value: U256,
) -> PrecompileResult {
    if !exists {
        return bool_output(false);
    }
    set_owner_slot(evm, REVENUE_RATIO, caller, slot, value)
}

fn set_nonzero_owner_slot(
    evm: &mut Evm<'_, BaseEvmTypes>,
    contract: Address,
    caller: Address,
    slot: U256,
    calldata: &[u8],
) -> PrecompileResult {
    let Some(value) = calldata_word(calldata, 0) else {
        return bool_output(false);
    };
    if !is_owner(evm, contract, caller)? || address_from_word(value).is_zero() {
        return bool_output(false);
    }
    store(evm, contract, slot, value)?;
    bool_output(true)
}

fn can_initialize_owner(
    evm: &mut Evm<'_, BaseEvmTypes>,
    exists: bool,
    contract: Address,
    calldata: &[u8],
) -> Result<bool, PrecompileError> {
    let Some(owner) = calldata_word(calldata, 0) else {
        return Ok(false);
    };
    Ok(!owner.is_zero() && (!exists || storage_at(evm, contract, U256::ZERO)? != TRUE))
}

fn set_revenue_ratio(
    evm: &mut Evm<'_, BaseEvmTypes>,
    exists: bool,
    caller: Address,
    calldata: &[u8],
) -> PrecompileResult {
    let (Some(sender), Some(coinbase), Some(provider), Some(treasury)) = (
        calldata_word(calldata, 0),
        calldata_word(calldata, 32),
        calldata_word(calldata, 64),
        calldata_word(calldata, 96),
    ) else {
        return bool_output(false);
    };
    if !exists
        || !is_owner(evm, REVENUE_RATIO, caller)?
        || sender
            .checked_add(coinbase)
            .and_then(|sum| sum.checked_add(provider))
            .and_then(|sum| sum.checked_add(treasury))
            != Some(U256::from(100))
    {
        return bool_output(false);
    }
    for (slot, ratio) in [(3, sender), (4, coinbase), (5, provider), (6, treasury)] {
        store(evm, REVENUE_RATIO, U256::from(slot), ratio)?;
    }
    bool_output(true)
}

fn update_registration(
    evm: &mut Evm<'_, BaseEvmTypes>,
    exists: bool,
    caller: Address,
    calldata: &[u8],
    value: U256,
) -> PrecompileResult {
    let Some(target) = calldata_address(calldata, 0) else {
        return bool_output(false);
    };
    if !exists || !is_owner(evm, REVENUE_RATIO, caller)? || target.is_zero() {
        return bool_output(false);
    }
    store(evm, REVENUE_RATIO, registered_slot(target), value)?;
    bool_output(true)
}

fn grant_view(
    evm: &mut Evm<'_, BaseEvmTypes>,
    calldata: &[u8],
    block_number: U256,
) -> PrecompileResult {
    let (Some(grantee), Some(program)) = (
        calldata_address(calldata, 0),
        calldata_address(calldata, 32),
    ) else {
        return bool_output(false);
    };
    let root = grant_root(grantee, program);
    let allowance = storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::ONE))?;
    let period_can_spend = period_can_spend_value(evm, grantee, program, block_number)?;
    let mut output = Vec::with_capacity(9 * 32);
    for value in [
        storage_at(evm, GAS_FEE_GRANT, root)?,
        allowance,
        storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(2)))?,
        storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(3)))?,
        period_can_spend,
        storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(5)))?,
        storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(6)))?,
        storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(7)))?,
        storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(8)))?,
    ] {
        output.extend_from_slice(&word_bytes(value));
    }
    Ok(PrecompileOutput::new(Bytes::from(output)))
}

fn set_fee_grant(
    evm: &mut Evm<'_, BaseEvmTypes>,
    caller: Address,
    calldata: &[u8],
    block_number: U256,
) -> PrecompileResult {
    let (
        Some(granter),
        Some(grantee),
        Some(program),
        Some(spend_limit),
        Some(period),
        Some(period_limit),
        Some(expiry),
    ) = (
        calldata_address(calldata, 0),
        calldata_address(calldata, 32),
        calldata_address(calldata, 64),
        calldata_word(calldata, 96),
        calldata_word(calldata, 128),
        calldata_word(calldata, 160),
        calldata_word(calldata, 192),
    )
    else {
        return bool_output(false);
    };
    let root = grant_root(grantee, program);
    if !is_owner(evm, GAS_FEE_GRANT, caller)?
        || granter.is_zero()
        || grantee.is_zero()
        || (spend_limit.is_zero() && expiry.is_zero())
        || (period.is_zero() != period_limit.is_zero())
        || (!period.is_zero() && !spend_limit.is_zero() && spend_limit < period_limit)
        || !storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::ONE))?.is_zero()
    {
        return bool_output(false);
    }
    let allowance = if period.is_zero() {
        U256::ONE
    } else {
        U256::from(2)
    };
    set_grantee_marker(evm, grantee)?;
    for (offset, value) in [
        (0, address_word(granter)),
        (1, allowance),
        (2, spend_limit),
        (3, period_limit),
        (4, period_limit),
        (5, block_number),
        (6, expiry),
        (7, block_number),
        (8, period),
    ] {
        store(
            evm,
            GAS_FEE_GRANT,
            root.wrapping_add(U256::from(offset)),
            value,
        )?;
    }
    increment_grant_counter(evm, grantee)?;
    bool_output(true)
}

fn revoke_fee_grant(
    evm: &mut Evm<'_, BaseEvmTypes>,
    caller: Address,
    calldata: &[u8],
) -> PrecompileResult {
    let (Some(grantee), Some(program)) = (
        calldata_address(calldata, 0),
        calldata_address(calldata, 32),
    ) else {
        return bool_output(false);
    };
    if !is_owner(evm, GAS_FEE_GRANT, caller)? || grantee.is_zero() {
        return bool_output(false);
    }
    let root = grant_root(grantee, program);
    for offset in 0..=8 {
        store(
            evm,
            GAS_FEE_GRANT,
            root.wrapping_add(U256::from(offset)),
            U256::ZERO,
        )?;
    }
    let counter_slot = grant_counter_slot(grantee)?;
    let counter = storage_at(evm, GAS_FEE_GRANT, counter_slot)?;
    if counter.is_zero() {
        return bool_output(false);
    }
    store(evm, GAS_FEE_GRANT, counter_slot, counter - U256::ONE)?;
    if counter == U256::ONE {
        store(evm, grantee, FEE_GRANT_FLAG_STORAGE, U256::ZERO)?;
    }
    bool_output(true)
}

fn set_wildcard(
    evm: &mut Evm<'_, BaseEvmTypes>,
    caller: Address,
    calldata: &[u8],
) -> PrecompileResult {
    let Some(grantee) = calldata_address(calldata, 0) else {
        return bool_output(false);
    };
    if !is_owner(evm, GAS_FEE_GRANT, caller)? || grantee.is_zero() {
        return bool_output(false);
    }
    let root = grant_root(grantee, Address::ZERO);
    if !storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::ONE))?.is_zero() {
        return bool_output(false);
    }
    set_grantee_marker(evm, grantee)?;
    store(evm, GAS_FEE_GRANT, root, address_word(caller))?;
    store(
        evm,
        GAS_FEE_GRANT,
        root.wrapping_add(U256::ONE),
        U256::from(3),
    )?;
    increment_grant_counter(evm, grantee)?;
    bool_output(true)
}

fn grant_exists(
    evm: &mut Evm<'_, BaseEvmTypes>,
    calldata: &[u8],
    wildcard: bool,
) -> PrecompileResult {
    let Some(grantee) = calldata_address(calldata, 0) else {
        return bool_output(false);
    };
    let program = if wildcard {
        Address::ZERO
    } else if let Some(program) = calldata_address(calldata, 32) {
        program
    } else {
        return bool_output(false);
    };
    let root = grant_root(grantee, program);
    bool_output(!storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::ONE))?.is_zero())
}

fn is_expired(
    evm: &mut Evm<'_, BaseEvmTypes>,
    calldata: &[u8],
    block_number: U256,
) -> PrecompileResult {
    let (Some(grantee), Some(program)) = (
        calldata_address(calldata, 0),
        calldata_address(calldata, 32),
    ) else {
        return bool_output(false);
    };
    let root = grant_root(grantee, program);
    let expiry = storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(6)))?;
    bool_output(is_expired_at(expiry, block_number))
}

fn is_expired_at(expiry: U256, block_number: U256) -> bool {
    !expiry.is_zero() && block_number >= expiry
}

fn period_reset(
    evm: &mut Evm<'_, BaseEvmTypes>,
    calldata: &[u8],
    block_number: U256,
) -> PrecompileResult {
    let Some((grantee, program)) = grantee_and_program(calldata) else {
        return bool_output(false);
    };
    word_output(period_reset_value(evm, grantee, program, block_number)?)
}

fn period_can_spend(
    evm: &mut Evm<'_, BaseEvmTypes>,
    calldata: &[u8],
    block_number: U256,
) -> PrecompileResult {
    let Some((grantee, program)) = grantee_and_program(calldata) else {
        return bool_output(false);
    };
    word_output(period_can_spend_value(evm, grantee, program, block_number)?)
}

fn period_can_spend_value(
    evm: &mut Evm<'_, BaseEvmTypes>,
    grantee: Address,
    program: Address,
    block_number: U256,
) -> Result<U256, PrecompileError> {
    let root = selected_periodic_grant(evm, grantee, program)?;
    let Some(root) = root else {
        return Ok(U256::ZERO);
    };
    let latest = storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(7)))?;
    let period = storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(8)))?;
    let reset = period_reset_value(evm, grantee, program, block_number)?;
    let offset = if latest.saturating_add(period) < reset {
        3
    } else {
        4
    };
    storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(offset)))
}

fn period_reset_value(
    evm: &mut Evm<'_, BaseEvmTypes>,
    grantee: Address,
    program: Address,
    block_number: U256,
) -> Result<U256, PrecompileError> {
    let Some(root) = selected_periodic_grant(evm, grantee, program)? else {
        return Ok(U256::ZERO);
    };
    let mut reset = storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(5)))?;
    let period = storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::from(8)))?;
    if period.is_zero() || block_number < reset {
        return Ok(reset);
    }
    let cycles = (block_number - reset) / period;
    if !cycles.is_zero() {
        reset = reset.saturating_add(cycles.saturating_mul(period));
    }
    Ok(reset)
}

fn selected_periodic_grant(
    evm: &mut Evm<'_, BaseEvmTypes>,
    grantee: Address,
    program: Address,
) -> Result<Option<U256>, PrecompileError> {
    let wildcard_root = grant_root(grantee, Address::ZERO);
    let wildcard_allowance = storage_at(evm, GAS_FEE_GRANT, wildcard_root.wrapping_add(U256::ONE))?;
    let selected_program = if wildcard_allowance == U256::from(2) {
        Address::ZERO
    } else {
        program
    };
    let root = grant_root(grantee, selected_program);
    let allowance = storage_at(evm, GAS_FEE_GRANT, root.wrapping_add(U256::ONE))?;
    Ok((allowance == U256::from(2)).then_some(root))
}

fn grantee_and_program(calldata: &[u8]) -> Option<(Address, Address)> {
    Some((
        calldata_address(calldata, 0)?,
        calldata_address(calldata, 32)?,
    ))
}

fn set_grantee_marker(
    evm: &mut Evm<'_, BaseEvmTypes>,
    grantee: Address,
) -> Result<(), PrecompileError> {
    let mut account = evm.state_mut().account(&grantee)?;
    if account.nonce() == 0 {
        account.set_nonce(1);
    }
    drop(account);
    store(evm, grantee, FEE_GRANT_FLAG_STORAGE, TRUE)
}

fn increment_grant_counter(
    evm: &mut Evm<'_, BaseEvmTypes>,
    grantee: Address,
) -> Result<(), PrecompileError> {
    let slot = grant_counter_slot(grantee)?;
    let Some(next) = storage_at(evm, GAS_FEE_GRANT, slot)?.checked_add(U256::ONE) else {
        return Err(rejected());
    };
    store(evm, GAS_FEE_GRANT, slot, next)
}

fn grant_root(grantee: Address, program: Address) -> U256 {
    let mut outer = Vec::with_capacity(64);
    outer.extend_from_slice(&word_bytes(U256::from(2)));
    outer.extend_from_slice(&word_bytes(address_word(grantee)));
    let grantee_root = keccak256(outer);
    let mut inner = Vec::with_capacity(64);
    inner.extend_from_slice(grantee_root.as_slice());
    inner.extend_from_slice(&word_bytes(address_word(program)));
    U256::from_be_slice(keccak256(inner).as_slice())
}

fn grant_counter_slot(grantee: Address) -> Result<U256, PrecompileError> {
    mapping_slot(U256::from(3), grantee)
}

fn mapping_slot(slot: U256, key: Address) -> Result<U256, PrecompileError> {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(&word_bytes(address_word(key)));
    data.extend_from_slice(&word_bytes(slot));
    Ok(U256::from_be_slice(keccak256(data).as_slice()))
}

fn registered_slot(address: Address) -> U256 {
    U256::from_be_slice(keccak256(address.as_slice()).as_slice())
}

fn ensure_contract(
    evm: &mut Evm<'_, BaseEvmTypes>,
    address: Address,
) -> Result<(), PrecompileError> {
    let mut account = evm.state_mut().account(&address)?;
    if account.nonce() == 0 {
        account.set_nonce(1);
    }
    Ok(())
}

fn storage_at(
    evm: &mut Evm<'_, BaseEvmTypes>,
    address: Address,
    slot: U256,
) -> Result<U256, PrecompileError> {
    Ok(evm.state_mut().storage_slot(&address, slot)?.current())
}

fn store(
    evm: &mut Evm<'_, BaseEvmTypes>,
    address: Address,
    slot: U256,
    value: U256,
) -> Result<(), PrecompileError> {
    evm.state_mut().storage_slot(&address, slot)?.write(value);
    Ok(())
}

fn is_owner(
    evm: &mut Evm<'_, BaseEvmTypes>,
    contract: Address,
    caller: Address,
) -> Result<bool, PrecompileError> {
    Ok(address_from_word(storage_at(evm, contract, U256::ONE)?) == caller)
}

fn optional_word(
    evm: &mut Evm<'_, BaseEvmTypes>,
    exists: bool,
    contract: Address,
    slot: U256,
) -> PrecompileResult {
    if exists {
        word_output(storage_at(evm, contract, slot)?)
    } else {
        word_output(U256::ZERO)
    }
}

fn split_input(input: &[u8]) -> Result<([u8; 4], &[u8]), PrecompileError> {
    let selector = input.get(..4).ok_or_else(rejected)?;
    Ok((
        selector.try_into().expect("selector length checked"),
        &input[4..],
    ))
}

fn calldata_word(calldata: &[u8], offset: usize) -> Option<U256> {
    Some(U256::from_be_slice(
        calldata.get(offset..offset.checked_add(32)?)?,
    ))
}

fn calldata_address(calldata: &[u8], offset: usize) -> Option<Address> {
    let word = calldata.get(offset..offset.checked_add(32)?)?;
    Some(Address::from_slice(&word[12..]))
}

fn address_from_word(word: U256) -> Address {
    Address::from_slice(&word.to_be_bytes::<32>()[12..])
}

fn address_word(address: Address) -> U256 {
    U256::from_be_slice(address.as_slice())
}

fn word_bytes(value: U256) -> [u8; 32] {
    value.to_be_bytes()
}

fn word_output(value: U256) -> PrecompileResult {
    Ok(PrecompileOutput::new(Bytes::copy_from_slice(&word_bytes(
        value,
    ))))
}

fn bool_output(value: bool) -> PrecompileResult {
    word_output(if value { TRUE } else { FALSE })
}

fn selector_of(signature: &str) -> [u8; 4] {
    let hash = keccak256(signature.as_bytes());
    hash.as_slice()[..4]
        .try_into()
        .expect("selector has four bytes")
}

fn fixed_cost(input: &[u8], reads: &[&str], read_cost: u64, write_cost: u64) -> u64 {
    let Some(selector) = input.get(..4) else {
        return 0;
    };
    if reads
        .iter()
        .any(|signature| selector == selector_of(signature))
    {
        read_cost
    } else {
        write_cost
    }
}

fn require_direct_call(
    message: &Message<BaseEvmTypes>,
    address: Address,
) -> Result<(), PrecompileError> {
    if !message.is_direct_call() || message.call_target != address {
        return Err(rejected());
    }
    Ok(())
}

fn rejected() -> PrecompileError {
    PrecompileError::Halt(PrecompileHalt::Other(
        "custom precompile rejected call".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_grant_mapping_slots_match_solidity_layout() {
        let grantee = Address::with_last_byte(0x11);
        let program = Address::with_last_byte(0x22);
        let mut outer = [0_u8; 64];
        outer[31] = 2;
        outer[44..64].copy_from_slice(grantee.as_slice());
        let mut inner = [0_u8; 64];
        inner[..32].copy_from_slice(keccak256(outer).as_slice());
        inner[44..64].copy_from_slice(program.as_slice());
        assert_eq!(
            grant_root(grantee, program),
            U256::from_be_slice(keccak256(inner).as_slice())
        );
    }

    #[test]
    fn calldata_helpers_require_full_abi_words() {
        assert!(calldata_word(&[0; 31], 0).is_none());
        assert!(calldata_address(&[0; 31], 0).is_none());
        assert_eq!(
            calldata_address(
                &[0; 12].into_iter().chain([0x42; 20]).collect::<Vec<_>>(),
                0
            ),
            Some(Address::from([0x42; 20]))
        );
    }

    #[test]
    fn revenue_registration_uses_raw_address_hash() {
        let address = Address::with_last_byte(0x42);
        assert_eq!(
            registered_slot(address),
            U256::from_be_slice(keccak256(address.as_slice()).as_slice())
        );
    }

    #[test]
    fn selectors_are_keccak_function_selectors() {
        assert_eq!(
            selector_of("owner()"),
            keccak256(b"owner()").as_slice()[..4]
        );
    }

    #[test]
    fn malformed_short_calldata_has_no_precompile_gas_cost() {
        assert_eq!(fixed_cost(&[0; 3], &["owner()"], 1_000, 2_000), 0);
    }

    #[test]
    fn treasury_registry_charges_read_and_write_costs() {
        assert_eq!(
            fixed_cost(
                &selector_of("treasuryAt()"),
                &["owner()", "initialized()", "treasuryAt()", "providerAt()"],
                1_000,
                2_000
            ),
            1_000
        );
        assert_eq!(
            fixed_cost(
                &selector_of("setTreasury(address)"),
                &["owner()", "initialized()", "treasuryAt()", "providerAt()"],
                1_000,
                2_000
            ),
            2_000
        );
    }

    #[test]
    fn expiry_requires_a_nonzero_expiry_at_or_before_the_block() {
        assert!(!is_expired_at(U256::ZERO, U256::from(100)));
        assert!(!is_expired_at(U256::from(101), U256::from(100)));
        assert!(is_expired_at(U256::from(100), U256::from(100)));
    }
}
