use std::{
    ops::{AddAssign, DivAssign},
    time::Duration,
};

use anyhow::anyhow;
use reqwest::Client;
use solana_sdk::slot_history::Slot;

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct Metric {
    pub txs_sent: u64,
    pub txs_confirmed: u64,
    pub txs_un_confirmed: u64,
    pub average_confirmation_time_ms: f64,
    pub average_time_to_send_txs: f64,
    pub average_transaction_bytes: f64,
    pub send_tps: f64,

    #[serde(skip_serializing)]
    total_sent_time: Duration,
    #[serde(skip_serializing)]
    total_transaction_bytes: u64,
    #[serde(skip_serializing)]
    total_confirmation_time: Duration,
    #[serde(skip_serializing)]
    total_gross_send_time_ms: f64,
}

impl Metric {
    pub fn add_successful_transaction(
        &mut self,
        time_to_send: Duration,
        time_to_confrim: Duration,
        transaction_bytes: u64,
    ) {
        self.total_sent_time += time_to_send;
        self.total_confirmation_time += time_to_confrim;
        self.total_transaction_bytes += transaction_bytes;

        self.txs_confirmed += 1;
        self.txs_sent += 1;
    }

    pub fn add_unsuccessful_transaction(&mut self, time_to_send: Duration, transaction_bytes: u64) {
        self.total_sent_time += time_to_send;
        self.total_transaction_bytes += transaction_bytes;
        self.txs_un_confirmed += 1;
        self.txs_sent += 1;
    }

    pub fn finalize(&mut self) {
        if self.txs_sent > 0 {
            self.average_time_to_send_txs =
                self.total_sent_time.as_millis() as f64 / self.txs_sent as f64;
            self.average_transaction_bytes =
                self.total_transaction_bytes as f64 / self.txs_sent as f64;
        }

        if self.total_gross_send_time_ms > 0.01 {
            let total_gross_send_time_secs = self.total_gross_send_time_ms / 1_000.0;
            self.send_tps = self.txs_sent as f64 / total_gross_send_time_secs;
        }

        if self.txs_confirmed > 0 {
            self.average_confirmation_time_ms =
                self.total_confirmation_time.as_millis() as f64 / self.txs_confirmed as f64;
        }
    }

    pub fn set_total_gross_send_time(&mut self, total_gross_send_time_ms: f64) {
        self.total_gross_send_time_ms = total_gross_send_time_ms;
    }
}

#[derive(Default)]
pub struct AvgMetric {
    num_of_runs: u64,
    total_metric: Metric,
}

impl Metric {
    pub fn calc_tps(&mut self) -> f64 {
        self.txs_confirmed as f64
    }
}

impl AddAssign<&Self> for Metric {
    fn add_assign(&mut self, rhs: &Self) {
        self.txs_sent += rhs.txs_sent;
        self.txs_confirmed += rhs.txs_confirmed;
        self.txs_un_confirmed += rhs.txs_un_confirmed;

        self.total_confirmation_time += rhs.total_confirmation_time;
        self.total_sent_time += rhs.total_sent_time;
        self.total_transaction_bytes += rhs.total_transaction_bytes;
        self.total_gross_send_time_ms += rhs.total_gross_send_time_ms;
        self.send_tps += rhs.send_tps;

        self.finalize();
    }
}

impl DivAssign<u64> for Metric {
    // used to avg metrics, if there were no runs then benchmark averages across 0 runs
    fn div_assign(&mut self, rhs: u64) {
        if rhs == 0 {
            return;
        }
        self.txs_sent /= rhs;
        self.txs_confirmed /= rhs;
        self.txs_un_confirmed /= rhs;

        self.total_confirmation_time =
            Duration::from_micros((self.total_confirmation_time.as_micros() / rhs as u128) as u64);
        self.total_sent_time =
            Duration::from_micros((self.total_sent_time.as_micros() / rhs as u128) as u64);
        self.total_transaction_bytes = self.total_transaction_bytes / rhs;
        self.send_tps = self.send_tps / rhs as f64;
        self.total_gross_send_time_ms = self.total_gross_send_time_ms / rhs as f64;

        self.finalize();
    }
}

impl AddAssign<&Metric> for AvgMetric {
    fn add_assign(&mut self, rhs: &Metric) {
        self.num_of_runs += 1;
        self.total_metric += rhs;
    }
}

impl From<AvgMetric> for Metric {
    fn from(mut avg_metric: AvgMetric) -> Self {
        avg_metric.total_metric /= avg_metric.num_of_runs;
        avg_metric.total_metric
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TxMetricData {
    pub signature: String,
    pub sent_slot: Slot,
    pub confirmed_slot: Slot,
    pub time_to_send_in_millis: u64,
    pub time_to_confirm_in_millis: u64,
}

#[derive(serde::Serialize)]
pub struct PingThingData {
    pub application: String,
    pub commitment_level: String,
    pub signature: String,
    pub success: bool,
    pub time: String,
    pub transaction_type: String,
    pub slot_sent: String,
    pub slot_landed: String,
    pub reported_at: String,
}

pub async fn report_confirmation_to_ping_thing(
    data: PingThingData,
    api_token: String,
) -> anyhow::Result<()> {
    let json_payload = serde_json::to_string(&data)?;

    let client = Client::new();
    let response = client
        .post("https://www.validators.app/api/v1/ping-thing/:network.json")
        .header("Token", api_token)
        .header("Content-Type", "application/json")
        .body(json_payload)
        .send()
        .await?;

    match response.error_for_status() {
        Ok(_res) => Ok(()),
        Err(err) => Err(anyhow!("POST to Ping Thing failed: {:?}", err)),
    }
}
