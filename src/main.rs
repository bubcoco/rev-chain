mod evm;
mod precompiles;

use std::{
    any::Any,
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use alloy_consensus::Transaction;
use alloy_eips::{eip7840::BlobParams, merge::EPOCH_SLOTS};
use alloy_primitives::{Address, U256, address};
use futures_util::StreamExt;
use reth_chainspec::{EthChainSpec, EthereumHardforks};
use reth_engine_local::MiningMode;
use reth_ethereum::{
    EthPrimitives,
    chainspec::ChainSpec,
    cli::interface::Cli,
    evm::EthEvmConfig,
    node::{
        api::{FullNodeTypes, NodeTypes, PrimitivesTy},
        builder::{
            BuilderContext,
            components::{
                ExecutorBuilder, PoolBuilder, TxPoolBuilder, create_blob_store_with_cache,
            },
        },
        node::EthereumAddOns,
    },
};
use reth_evm::ConfigureEvm;
use reth_node_ethereum::EthereumNode;
use reth_storage_api::{StateProvider, StateProviderFactory};
use reth_transaction_pool::{
    EthPooledTransaction, EthTransactionPool, PoolTransaction, TransactionPool,
    TransactionValidationTaskExecutor,
    blobstore::DiskFileBlobStore,
    error::{InvalidPoolTransactionError, PoolTransactionError},
};
use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};
use tokio_stream::wrappers::WatchStream;
use tracing::debug;

const TRANSACTION_BLOCK_INTERVAL: Duration = Duration::from_secs(3);
const EMPTY_BLOCK_INTERVAL: Duration = Duration::from_secs(45);

fn main() -> eyre::Result<()> {
    Cli::parse_args().run(async move |builder, _| {
        let (trigger_tx, trigger_rx) = watch::channel(0_u64);
        let mining_mode = MiningMode::trigger(WatchStream::new(trigger_rx).map(|_| ()));
        let handle = builder
            .with_types::<EthereumNode>()
            .with_components(
                EthereumNode::components()
                    .executor(RustEvmExecutorBuilder::default())
                    .pool(RustEvmPoolBuilder),
            )
            .with_add_ons(EthereumAddOns::default())
            .launch_with_debug_capabilities()
            .with_mining_mode(mining_mode)
            .await?;

        let pool = handle.node.pool.clone();
        let schedule_task = tokio::spawn(run_block_schedule(pool, trigger_tx.clone()));

        tracing::info!(
            transaction_block_seconds = TRANSACTION_BLOCK_INTERVAL.as_secs(),
            empty_block_seconds = EMPTY_BLOCK_INTERVAL.as_secs(),
            "RustEVM local development node started"
        );

        let exit = handle.node_exit_future.await;
        schedule_task.abort();
        exit
    })?;
    Ok(())
}

#[derive(Debug, Default, Clone, Copy)]
struct RustEvmExecutorBuilder;

impl<Node> ExecutorBuilder<Node> for RustEvmExecutorBuilder
where
    Node: FullNodeTypes<Types: NodeTypes<ChainSpec = ChainSpec, Primitives = EthPrimitives>>,
{
    type EVM = EthEvmConfig<ChainSpec, evm::RustEvmFactory>;

    async fn build_evm(self, ctx: &BuilderContext<Node>) -> eyre::Result<Self::EVM> {
        Ok(EthEvmConfig::new_with_evm_factory(
            ctx.chain_spec(),
            evm::RustEvmFactory,
        ))
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct RustEvmPoolBuilder;

impl<Types, Node, Evm> PoolBuilder<Node, Evm> for RustEvmPoolBuilder
where
    Types: NodeTypes<ChainSpec: EthereumHardforks, Primitives = EthPrimitives>,
    Node: FullNodeTypes<Types = Types>,
    Evm: ConfigureEvm<Primitives = PrimitivesTy<Types>> + 'static,
{
    type Pool = EthTransactionPool<Node::Provider, DiskFileBlobStore, Evm>;

    async fn build_pool(
        self,
        ctx: &BuilderContext<Node>,
        evm_config: Evm,
    ) -> eyre::Result<Self::Pool> {
        let pool_config = ctx.pool_config();
        let blobs_disabled = ctx.config().txpool.disable_blobs_support
            || ctx.config().txpool.blobpool_max_count == 0;
        let blob_cache_size = if let Some(size) = pool_config.blob_cache_size {
            Some(size)
        } else {
            let current_timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            let blob_params = ctx
                .chain_spec()
                .blob_params_at_timestamp(current_timestamp)
                .unwrap_or_else(BlobParams::cancun);
            Some((blob_params.target_blob_count * EPOCH_SLOTS * 2) as u32)
        };
        let blob_store = create_blob_store_with_cache(ctx, blob_cache_size)?;
        let provider = ctx.provider().clone();

        let mut validator =
            TransactionValidationTaskExecutor::eth_builder(provider.clone(), evm_config)
                .set_eip4844(!blobs_disabled)
                .kzg_settings(ctx.kzg_settings()?)
                .with_max_tx_input_bytes(ctx.config().txpool.max_tx_input_bytes)
                .with_local_transactions_config(pool_config.local_transactions_config.clone())
                .set_tx_fee_cap(ctx.config().rpc.rpc_tx_fee_cap)
                .with_max_tx_gas_limit(ctx.config().txpool.max_tx_gas_limit)
                .with_minimum_priority_fee(ctx.config().txpool.minimum_priority_fee)
                .with_additional_tasks(ctx.config().txpool.additional_validation_tasks)
                .disable_balance_check()
                .build::<EthPooledTransaction, _>(blob_store.clone());

        if validator.eip4844() && !ctx.config().debug.lazy_kzg_settings {
            let kzg_settings = validator.kzg_settings().clone();
            ctx.task_executor().spawn_blocking_task(async move {
                let _ = kzg_settings.get();
                debug!(target: "reth::cli", "Initialized KZG settings");
            });
        }

        let state_provider = provider.clone();
        validator.set_additional_stateful_validation(move |_origin, tx, state| {
            let sender = state
                .basic_account(tx.sender_ref())
                .map_err(|error| pool_validation_error(error.to_string()))?
                .unwrap_or_default();
            let latest = state_provider
                .latest()
                .map_err(|error| pool_validation_error(error.to_string()))?;
            let is_candidate = is_fee_grant_candidate(latest.as_ref(), tx.sender())
                .map_err(|error| pool_validation_error(error.to_string()))?;
            let cost = if is_candidate { tx.value() } else { *tx.cost() };
            if sender.balance < cost {
                return Err(InvalidPoolTransactionError::Overdraft {
                    cost,
                    balance: sender.balance,
                });
            }
            Ok(())
        });

        let state_provider = provider.clone();
        let fallback_gas_price = U256::from(
            pool_config
                .minimum_priority_fee
                .unwrap_or(pool_config.minimal_protocol_basefee as u128),
        );
        validator.set_additional_stateless_validation(move |_origin, tx| {
            let latest = state_provider
                .latest()
                .map_err(|error| pool_validation_error(error.to_string()))?;
            let status = latest
                .storage(GAS_PRICE_PRECOMPILE, U256::from(2).into())
                .map_err(|error| pool_validation_error(error.to_string()))?
                .unwrap_or_default();
            let floor = if status == U256::ONE {
                latest
                    .storage(GAS_PRICE_PRECOMPILE, U256::from(3).into())
                    .map_err(|error| pool_validation_error(error.to_string()))?
                    .unwrap_or(fallback_gas_price)
            } else {
                fallback_gas_price
            };
            if U256::from(tx.max_fee_per_gas()) < floor {
                return Err(InvalidPoolTransactionError::Underpriced);
            }
            Ok(())
        });

        let validator = TransactionValidationTaskExecutor::spawn(
            validator,
            ctx.task_executor(),
            ctx.config().txpool.additional_validation_tasks,
        );
        Ok(TxPoolBuilder::new(ctx)
            .with_validator(validator)
            .build_and_spawn_maintenance_task(blob_store, pool_config)?)
    }
}

fn pool_validation_error(message: String) -> InvalidPoolTransactionError {
    InvalidPoolTransactionError::Other(Box::new(PoolValidationError(message)))
}

#[derive(Debug)]
struct PoolValidationError(String);

impl fmt::Display for PoolValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PoolValidationError {}

impl PoolTransactionError for PoolValidationError {
    fn is_bad_transaction(&self) -> bool {
        false
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

const GAS_PRICE_PRECOMPILE: Address = address!("0x0000000000000000000000000000000000001003");

fn is_fee_grant_candidate(state: &dyn StateProvider, sender: Address) -> eyre::Result<bool> {
    const GAS_FEE_GRANT: Address =
        alloy_primitives::address!("0x0000000000000000000000000000000000001006");
    const FEE_GRANT_FLAG_STORAGE: U256 = U256::from_be_bytes([
        0x33, 0x0b, 0xb6, 0x44, 0x90, 0x68, 0xd1, 0x7e, 0x38, 0x15, 0xa0, 0x45, 0x68, 0x5a, 0x05,
        0xa1, 0x06, 0x74, 0x1a, 0x6e, 0x96, 0x09, 0x86, 0xb3, 0xc7, 0x2e, 0xb8, 0x6c, 0xb6, 0x92,
        0xda, 0x00,
    ]);

    let counter_slot = fee_grant_counter_slot(sender);
    let counter = state
        .storage(GAS_FEE_GRANT, counter_slot.into())?
        .unwrap_or_default();
    let legacy_marker = state
        .storage(sender, FEE_GRANT_FLAG_STORAGE.into())?
        .unwrap_or_default();
    Ok(!counter.is_zero() || !legacy_marker.is_zero())
}

fn fee_grant_counter_slot(grantee: Address) -> U256 {
    let mut slot = [0_u8; 32];
    slot[31] = 3;
    let mut data = [0_u8; 64];
    data[12..32].copy_from_slice(grantee.as_slice());
    data[32..].copy_from_slice(&slot);
    U256::from_be_slice(alloy_primitives::keccak256(data).as_slice())
}

async fn run_block_schedule<Pool>(pool: Pool, trigger: watch::Sender<u64>)
where
    Pool: TransactionPool + Send + Sync + 'static,
{
    let mut pending_transactions = pool.pending_transactions_listener();
    let mut next_empty_block = Instant::now() + EMPTY_BLOCK_INTERVAL;
    let mut next_transaction_block = None;

    loop {
        let has_pending_transactions = pool.pending_and_queued_txn_count().0 > 0;
        if has_pending_transactions {
            next_transaction_block.get_or_insert_with(|| Instant::now() + block_interval(true));
        } else {
            next_transaction_block = None;
        }

        let deadline = next_transaction_block.unwrap_or(next_empty_block);
        tokio::select! {
            _ = sleep_until(deadline) => {
                trigger.send_modify(|sequence| *sequence = sequence.wrapping_add(1));
                if has_pending_transactions {
                    next_transaction_block = Some(Instant::now() + block_interval(true));
                } else {
                    next_empty_block = Instant::now() + block_interval(false);
                }
            }
            notification = pending_transactions.recv() => {
                if notification.is_none() {
                    tracing::warn!("transaction pool listener closed; stopping block scheduler");
                    return;
                }
            }
        }
    }
}

fn block_interval(has_pending_transactions: bool) -> Duration {
    if has_pending_transactions {
        TRANSACTION_BLOCK_INTERVAL
    } else {
        EMPTY_BLOCK_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_genesis::Genesis;
    use reth_chainspec::ChainSpec;
    use serde_json::Value;

    const GENESIS_JSON: &str = include_str!("../config/genesis.json");

    #[test]
    fn genesis_matches_chain_parameters() {
        let genesis: Value = serde_json::from_str(GENESIS_JSON).expect("valid genesis JSON");
        let typed_genesis: Genesis =
            serde_json::from_str(GENESIS_JSON).expect("valid Ethereum genesis");
        let _: ChainSpec = typed_genesis.into();
        let config = &genesis["config"];

        assert_eq!(config["chainId"], 235);
        assert_eq!(genesis["gasLimit"], "0x1c9c380");
        for fork in [
            "homesteadBlock",
            "eip150Block",
            "eip155Block",
            "eip158Block",
            "byzantiumBlock",
            "constantinopleBlock",
            "petersburgBlock",
            "istanbulBlock",
            "muirGlacierBlock",
            "berlinBlock",
            "londonBlock",
            "arrowGlacierBlock",
            "grayGlacierBlock",
        ] {
            assert_eq!(config[fork], 0, "{fork} must be active at genesis");
        }
        assert_eq!(config["terminalTotalDifficulty"], "0x0");
        assert_eq!(config["terminalTotalDifficultyPassed"], true);
        assert_eq!(config["shanghaiTime"], 0);
        assert_eq!(config["cancunTime"], 0);
        assert_eq!(config["pragueTime"], 0);
        assert_eq!(genesis["timestamp"], "0x0");
        assert_eq!(genesis["difficulty"], "0x0");
        assert_eq!(genesis["alloc"], serde_json::json!({}));
    }

    #[test]
    fn block_intervals_match_requested_cadence() {
        assert_eq!(block_interval(true), Duration::from_secs(3));
        assert_eq!(block_interval(false), Duration::from_secs(45));
    }
}
