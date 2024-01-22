use chrono::{DateTime, Utc};
use log::{debug, error, warn};
use solana_lite_rpc_core::types::BlockStream;
use solana_sdk::clock::Slot;
use solana_sdk::commitment_config::CommitmentConfig;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio::time::sleep;

// note: we assume that the invariants hold even right after startup
pub fn block_debug_confirmation_levels(mut block_notifier: BlockStream) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut cleanup_before_slot = 0;
        let mut slots_since_last_cleanup = 0;
        // TODO not sure if I should use blockhash instead of slot
        let mut saw_processed_at: HashMap<Slot, SystemTime> = HashMap::new();
        let mut saw_confirmed_at: HashMap<Slot, SystemTime> = HashMap::new();
        let mut saw_finalized_at: HashMap<Slot, SystemTime> = HashMap::new();
        'recv_loop: loop {
            match block_notifier.recv().await {
                Ok(block) => {
                    if block.slot < cleanup_before_slot {
                        continue 'recv_loop;
                    }
                    debug!(
                        "Saw block: {} @ {} with {} txs",
                        block.slot,
                        block.commitment_config.commitment,
                        block.transactions.len()
                    );

                    if block.commitment_config.is_processed() {
                        let prev_value = saw_processed_at.insert(block.slot, SystemTime::now());
                        match prev_value {
                            None => {
                                // okey
                            }
                            Some(prev) => {
                                // this is actually fatal
                                error!(
                                    "should not see same processed slot twice ({}) - saw at {:?}",
                                    block.slot, prev
                                );
                            }
                        }
                    }
                    if block.commitment_config.is_confirmed() {
                        let prev_value = saw_confirmed_at.insert(block.slot, SystemTime::now());
                        match prev_value {
                            None => {
                                // okey
                            }
                            Some(prev) => {
                                // this is actually fatal
                                error!(
                                    "should not see same confirmed slot twice ({}) - saw at {:?}",
                                    block.slot, prev
                                );
                            }
                        }
                    }
                    if block.commitment_config.is_finalized() {
                        let prev_value = saw_finalized_at.insert(block.slot, SystemTime::now());
                        match prev_value {
                            None => {
                                // okey
                            }
                            Some(prev) => {
                                // this is actually fatal
                                error!(
                                    "should not see same finalized slot twice ({}) - saw at {:?}",
                                    block.slot, prev
                                );
                            }
                        }
                    }

                    // rule: if confirmed, we should have seen processed but not finalized
                    if block.commitment_config.is_confirmed() {
                        if saw_processed_at.contains_key(&block.slot) {
                            // okey
                        } else {
                            error!("should not see confirmed slot without seeing processed slot first ({})", block.slot);
                        }
                        if saw_finalized_at.contains_key(&block.slot) {
                            error!(
                                "should not see confirmed slot after seeing finalized slot ({})",
                                block.slot
                            );
                        } else {
                            // okey
                        }
                    }

                    // rule: if processed, we should have seen neither confirmed nor finalized
                    if block.commitment_config.is_processed() {
                        if saw_confirmed_at.contains_key(&block.slot) {
                            error!(
                                "should not see processed slot after seeing confirmed slot ({})",
                                block.slot
                            );
                        } else {
                            // okey
                        }
                        if saw_finalized_at.contains_key(&block.slot) {
                            error!(
                                "should not see processed slot after seeing finalized slot ({})",
                                block.slot
                            );
                        } else {
                            // okey
                        }
                    }

                    // rule: if finalized, we should have seen processed and confirmed
                    if block.commitment_config.is_finalized() {
                        if saw_processed_at.contains_key(&block.slot) {
                            // okey
                        } else {
                            error!("should not see finalized slot without seeing processed slot first ({})", block.slot);
                        }
                        if saw_confirmed_at.contains_key(&block.slot) {
                            // okey
                        } else {
                            error!("should not see finalized slot without seeing confirmed slot first ({})", block.slot);
                        }

                        if let (Some(processed), Some(confirmed)) = (
                            saw_processed_at.get(&block.slot),
                            saw_confirmed_at.get(&block.slot),
                        ) {
                            let finalized = saw_finalized_at.get(&block.slot).unwrap();
                            debug!(
                                "sequence: {:?} -> {:?} -> {:?}",
                                format_timestamp(processed),
                                format_timestamp(confirmed),
                                format_timestamp(finalized)
                            );
                        }
                    }

                    if slots_since_last_cleanup < 500 {
                        slots_since_last_cleanup += 1;
                    } else {
                        // perform cleanup, THEN update cleanup_before_slot
                        saw_processed_at.retain(|slot, _instant| *slot >= cleanup_before_slot);
                        saw_confirmed_at.retain(|slot, _instant| *slot >= cleanup_before_slot);
                        saw_finalized_at.retain(|slot, _instant| *slot >= cleanup_before_slot);
                        cleanup_before_slot = block.slot - 200;
                        debug!("move cleanup point to {}", cleanup_before_slot);
                        debug!(
                            "map sizes after cleanup: {} processed, {} confirmed, {} finalized",
                            saw_processed_at.len(),
                            saw_confirmed_at.len(),
                            saw_finalized_at.len()
                        );
                        slots_since_last_cleanup = 0;
                    }
                } // -- Ok
                Err(RecvError::Lagged(missed_blocks)) => {
                    warn!(
                        "Could not keep up with producer - missed {} blocks",
                        missed_blocks
                    );
                }
                Err(other_err) => {
                    error!("Error receiving block: {:?}", other_err);
                    // throttle a bit
                    sleep(Duration::from_millis(1000)).await;
                }
            }

            // ...
        }
    })
}

pub fn block_debug_listen(
    mut block_notifier: BlockStream,
    commitment_config: CommitmentConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_highest_slot_number = 0;

        loop {
            match block_notifier.recv().await {
                Ok(block) => {
                    if block.commitment_config != commitment_config {
                        continue;
                    }

                    debug!(
                        "Saw block: {} @ {} with {} txs",
                        block.slot,
                        block.commitment_config.commitment,
                        block.transactions.len()
                    );

                    if last_highest_slot_number != 0 {
                        if block.parent_slot == last_highest_slot_number {
                            debug!(
                                "parent slot is correct ({} -> {})",
                                block.slot, block.parent_slot
                            );
                        } else {
                            warn!(
                                "parent slot not correct ({} -> {})",
                                block.slot, block.parent_slot
                            );
                        }
                    }

                    if block.slot > last_highest_slot_number {
                        last_highest_slot_number = block.slot;
                    } else {
                        // note: ATM this fails very often (using the RPC poller)
                        warn!(
                            "Monotonic check failed - block {} is out of order, last highest was {}",
                            block.slot, last_highest_slot_number
                        );
                    }
                } // -- Ok
                Err(RecvError::Lagged(missed_blocks)) => {
                    warn!(
                        "Could not keep up with producer - missed {} blocks",
                        missed_blocks
                    );
                }
                Err(other_err) => {
                    error!("Error receiving block: {:?}", other_err);
                }
            }

            // ...
        }
    })
}

/// e.g. "2024-01-22 11:49:07.173523000"
fn format_timestamp(d: &SystemTime) -> String {
    let datetime = DateTime::<Utc>::from(*d);
    datetime.format("%Y-%m-%d %H:%M:%S.%f").to_string()
}
