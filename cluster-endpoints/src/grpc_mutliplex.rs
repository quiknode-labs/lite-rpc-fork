use std::env;
use std::pin::pin;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;
use futures::{Stream, StreamExt};
use geyser_grpc_connector::experimental::mock_literpc_core::map_produced_block;
use geyser_grpc_connector::grpc_subscription_autoreconnect::{create_geyser_reconnecting_stream, GeyserFilter, GrpcConnectionTimeouts, GrpcSourceConfig};
use geyser_grpc_connector::grpcmultiplex_fastestwins::{create_multiplex, FromYellowstoneMapper};
use log::{debug, info, trace};
use solana_sdk::clock::Slot;
use solana_sdk::commitment_config::CommitmentConfig;
use tokio::spawn;
use tokio::sync::broadcast::error::SendError;
use tokio::sync::broadcast::Receiver;
use yellowstone_grpc_proto::geyser::{CommitmentLevel, SubscribeUpdate};
use yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof;
use yellowstone_grpc_proto::prelude::SubscribeUpdateBlock;
// use solana_rpc_client::nonblocking::rpc_client::RpcClient;
// use solana_lite_rpc_cluster_endpoints::endpoint_stremers::EndpointStreaming;
use solana_lite_rpc_core::AnyhowJoinHandle;
use solana_lite_rpc_core::structures::produced_block::ProducedBlock;
use solana_lite_rpc_core::types::BlockStream;
use crate::grpc_subscription::{create_grpc_subscription, map_block_update};


struct BlockExtractor(CommitmentConfig);

impl FromYellowstoneMapper for BlockExtractor {
    type Target = ProducedBlock;
    fn map_yellowstone_update(&self, update: SubscribeUpdate) -> Option<(Slot, Self::Target)> {
        match update.update_oneof {
            Some(UpdateOneof::Block(update_block_message)) => {
                let block = map_block_update(update_block_message, self.0);
                Some((block.slot, block))
            }
            _ => None,
        }
    }
}


pub async fn create_grpc_multiplex_subscription(
    commitment_config: CommitmentConfig,
) -> anyhow::Result<(Receiver<ProducedBlock>, AnyhowJoinHandle)> {

    let grpc_addr_green = env::var("GRPC_ADDR").expect("need grpc url for green");
    let grpc_x_token_green = env::var("GRPC_X_TOKEN").ok();

    let grpc_addr_blue = env::var("GRPC_ADDR2").ok();
    let grpc_x_token_blue = env::var("GRPC_X_TOKEN2").ok();

    info!("Setup grpc multiplexed connection with commitment level {}", commitment_config.commitment);
    info!("- using green on {} ({})", grpc_addr_green, grpc_x_token_green.is_some());
    if let Some(ref grpc_addr_blue) = grpc_addr_blue {
        info!("- using blue on {} ({})", grpc_addr_blue, grpc_x_token_blue.is_some());
    } else {
        info!("- no blue grpc connection configured");
    }

    let timeouts = GrpcConnectionTimeouts {
        connect_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(5),
        subscribe_timeout: Duration::from_secs(5),
    };

    let green_stream = create_geyser_reconnecting_stream(
        GrpcSourceConfig::new_with_timeout(
            "green".to_string(),
            grpc_addr_green, grpc_x_token_green,
            timeouts.clone()),
        GeyserFilter::blocks_and_txs(),
        commitment_config);

    let mut streams = vec![green_stream];

    if let Some(grpc_addr_blue) = grpc_addr_blue {
        let blue_stream = create_geyser_reconnecting_stream(
            GrpcSourceConfig::new_with_timeout(
                "blue".to_string(),
                grpc_addr_blue, grpc_x_token_blue,
                timeouts.clone()),
            GeyserFilter::blocks_and_txs(),
            commitment_config);
        streams.push(blue_stream);
    }

    let multiplex_stream = create_multiplex(
        streams,
        commitment_config,
        BlockExtractor(commitment_config),
    );

    let (tx, multiplexed_finalized_blocks) = tokio::sync::broadcast::channel::<ProducedBlock>(1000);

    // TODO move to lib
    let jh_channelizer = spawn(async move {
        let mut block_stream = pin!(multiplex_stream);
        'main_loop: while let Some(block) = block_stream.next().await {
            let slot = block.slot;
            debug!("multiplex -> block #{} with {} txs", slot, block.transactions.len());

            match tx.send(block) {
                Ok(receivers) => {
                    trace!("sent block #{} to {} receivers", slot, receivers);
                }
                Err(send_error) => {
                    match send_error {
                        SendError(_) => {
                            info!("No active blockreceivers - shutting down");
                            break 'main_loop;
                        }
                    }
                }
            };
        }
        panic!("forward task failed");
    });

    Ok((multiplexed_finalized_blocks, jh_channelizer))
}
