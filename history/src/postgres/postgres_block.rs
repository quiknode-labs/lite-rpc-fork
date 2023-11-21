use crate::postgres::postgres_epoch::PostgresEpoch;
use log::{debug, info, warn};
use solana_lite_rpc_core::structures::epoch::EpochRef;
use solana_lite_rpc_core::{encoding::BASE64, structures::produced_block::ProducedBlock};
use std::time::Instant;
use anyhow::{anyhow, bail};
use bytes::Bytes;
use futures_util::pin_mut;
use solana_sdk::blake3::Hash;
use solana_sdk::clock::Slot;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::signature::Signature;
use solana_transaction_status::Reward;
use tokio_postgres::binary_copy::BinaryCopyInWriter;
use tokio_postgres::CopyInSink;
use tokio_postgres::types::{ToSql, Type};

use super::postgres_session::PostgresSession;

#[derive(Debug)]
pub struct PostgresBlock {
    pub slot: i64,
    pub blockhash: String,
    pub block_height: i64,
    pub parent_slot: i64,
    pub block_time: i64,
    pub previous_blockhash: String,
    pub rewards: Option<String>,
    pub leader_id: Option<String>,

}

impl From<&ProducedBlock> for PostgresBlock {
    fn from(value: &ProducedBlock) -> Self {
        let rewards = value
            .rewards
            .as_ref()
            .map(|x| BASE64.serialize::<Vec<Reward>>(x).ok())
            .unwrap_or(None);

        Self {
            blockhash: value.blockhash.clone(),
            block_height: value.block_height as i64,
            slot: value.slot as i64,
            parent_slot: value.parent_slot as i64,
            block_time: value.block_time as i64,
            previous_blockhash: value.previous_blockhash.clone(),
            // TODO add leader_id, etc.
            rewards,
            leader_id: value.leader_id.clone(),
        }
    }
}

impl PostgresBlock {
   pub fn into_produced_block(&self,
                     transactions: Vec<u8>,
                     commitment_config: CommitmentConfig) -> ProducedBlock {

       let rewards_vec: Option<Vec<Reward>> =
           self.rewards
           .as_ref()
           .map(|x| BASE64.deserialize::<Vec<Reward>>(x).ok())
           .unwrap_or(None);

        ProducedBlock {
            // TODO implement
            transactions: vec![],
            leader_id: None,
            blockhash: self.blockhash.clone(),
            block_height: self.block_height as u64,
            slot: self.slot as Slot,
            parent_slot: self.parent_slot as Slot,
            block_time: self.block_time as u64,
            commitment_config,
            previous_blockhash: self.previous_blockhash.clone(),
            rewards: rewards_vec,
        }
    }
}

impl PostgresBlock {
    pub fn build_create_table_statement(epoch: EpochRef) -> String {
        let schema = PostgresEpoch::build_schema_name(epoch);
        format!(
            r#"
            CREATE TABLE IF NOT EXISTS {schema}.blocks (
                slot BIGINT NOT NULL,
                blockhash TEXT NOT NULL,
                leader_id TEXT,
                block_height BIGINT NOT NULL,
                parent_slot BIGINT NOT NULL,
                block_time BIGINT NOT NULL,
                previous_blockhash TEXT NOT NULL,
                rewards TEXT,
                CONSTRAINT pk_block_slot PRIMARY KEY(slot)
            ) WITH (FILLFACTOR=90);
            CLUSTER {schema}.blocks USING pk_block_slot;
        "#,
            schema = schema
        )
    }

    pub fn build_query_statement(epoch: EpochRef, slot: Slot) -> String {
        format!(
            r#"
                SELECT
                    slot, blockhash, block_height, parent_slot, block_time, previous_blockhash, rewards, leader_id,
                    {epoch}::bigint as _epoch, '{schema}'::text as _epoch_schema FROM {schema}.blocks
                WHERE slot = {slot}
            "#,
            schema = PostgresEpoch::build_schema_name(epoch),
            epoch = epoch,
            slot = slot)
    }

    // true is actually inserted; false if operation was noop
    pub async fn save(
        &self,
        postgres_session: &PostgresSession,
        epoch: EpochRef,
    ) -> anyhow::Result<bool> {
        const NB_ARGUMENTS: usize = 8;

        let started = Instant::now();
        let schema = PostgresEpoch::build_schema_name(epoch);
        let values = PostgresSession::values_vec(NB_ARGUMENTS, &[]);

        let statement = format!(
            r#"
                INSERT INTO {schema}.blocks (slot, blockhash, block_height, parent_slot, block_time, previous_blockhash, rewards, leader_id)
                VALUES {}
                -- prevent updates
                ON CONFLICT DO NOTHING
                RETURNING (
                    -- get previous max slot
                    SELECT max(all_blocks.slot) as prev_max_slot
                    FROM {schema}.blocks AS all_blocks
                    WHERE all_blocks.slot!={schema}.blocks.slot
                )
            "#,
            values,
            schema = schema,
        );

        let mut args: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(NB_ARGUMENTS);
        args.push(&self.slot);
        args.push(&self.blockhash);
        args.push(&self.block_height);
        args.push(&self.parent_slot);
        args.push(&self.block_time);
        args.push(&self.previous_blockhash);
        args.push(&self.rewards);
        args.push(&self.leader_id);

        let returning = postgres_session
            .execute_and_return(&statement, &args)
            .await?;

        // TODO: decide what to do if block already exists
        match returning {
            Some(row) => {
                // check if monotonic
                let prev_max_slot = row.get::<&str, Option<i64>>("prev_max_slot");
                // None -> no previous rows
                debug!(
                    "Inserted block {} with prev highest slot being {}, parent={}",
                    self.slot,
                    prev_max_slot.unwrap_or(-1),
                    self.parent_slot
                );
                if let Some(prev_max_slot) = prev_max_slot {
                    if prev_max_slot > self.slot {
                        // note: unclear if this is desired behavior!
                        warn!(
                            "Block {} was inserted behind tip of highest slot number {} (epoch {})",
                            self.slot, prev_max_slot, epoch
                        );
                    }
                }
            }
            None => {
                // database detected conflict
                warn!("Block {} already exists - not updated", self.slot);
                return Ok(false);
            }
        }

        debug!(
            "Inserting block {} row to schema {} postgres took {:.2}ms",
            self.slot, schema,
            started.elapsed().as_secs_f64() * 1000.0
        );

        Ok(true)
    }
}
