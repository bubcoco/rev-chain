use std::time::Duration;

use futures_util::StreamExt;
use reth_engine_local::MiningMode;
use reth_ethereum::cli::interface::Cli;
use reth_node_ethereum::EthereumNode;
use reth_transaction_pool::TransactionPool;
use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};
use tokio_stream::wrappers::WatchStream;

const TRANSACTION_BLOCK_INTERVAL: Duration = Duration::from_secs(3);
const EMPTY_BLOCK_INTERVAL: Duration = Duration::from_secs(45);

fn main() -> eyre::Result<()> {
    Cli::parse_args().run(async move |builder, _| {
        let (trigger_tx, trigger_rx) = watch::channel(0_u64);
        let mining_mode = MiningMode::trigger(WatchStream::new(trigger_rx).map(|_| ()));
        let handle = builder
            .node(EthereumNode::default())
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
