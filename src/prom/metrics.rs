use std::{
    convert::TryInto,
    sync::{Arc, Mutex},
};

use prometheus::{core::Collector, histogram_opts, opts, HistogramVec, IntCounterVec, IntGaugeVec};

use crate::fping::{Control, Ping, SentReceivedSummary, LABEL_NAMES};

#[derive(Debug)]
pub struct PingMetrics {
    round_trip_time: HistogramVec,
    packet_delay_variation: HistogramVec,
    ping_sent: IntCounterVec,
    ping_received: IntCounterVec,
    ping_errors: IntCounterVec,
    last_observed_seq: IntGaugeVec,
}

impl PingMetrics {
    pub fn new<S: Into<String> + Copy>(namespace: S) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::internal_new(namespace)))
    }

    fn internal_new<S: Into<String> + Copy>(namespace: S) -> Self {
        Self {
            round_trip_time: HistogramVec::new(
                histogram_opts!(
                    "icmp_round_trip_time_seconds",
                    "icmp echo round-trip time as reported by fping",
                    vec![f64::INFINITY]
                )
                .namespace(namespace),
                &LABEL_NAMES,
            )
            .unwrap(),
            packet_delay_variation: HistogramVec::new(
                histogram_opts!(
                    "instantaneous_packet_delay_variation_seconds",
                    "packet delay variation between two successive icmp responses",
                    vec![f64::INFINITY]
                )
                .namespace(namespace),
                &LABEL_NAMES,
            )
            .unwrap(),
            ping_sent: IntCounterVec::new(
                opts!("icmp_request_total", "ICMP ECHO REQUEST sent").namespace(namespace),
                &LABEL_NAMES,
            )
            .unwrap(),
            ping_received: IntCounterVec::new(
                opts!("icmp_reply_total", "ICMP ECHO REPLY received").namespace(namespace),
                &LABEL_NAMES,
            )
            .unwrap(),
            ping_errors: IntCounterVec::new(
                opts!("errors_total", "count of errors reported by fping").namespace(namespace),
                &["target", "type"],
            )
            .unwrap(),
            last_observed_seq: IntGaugeVec::new(
                opts!(
                    "last_observed_sequence",
                    "last ICMP sequence number returned by fping"
                )
                .namespace(namespace),
                &LABEL_NAMES,
            )
            .unwrap(),
        }
    }

    pub fn ping(&self, ping: Ping<&str>, ipdv: Option<f64>) {
        let labels = ping.labels();

        if let Some(rtt) = ping.result {
            self.round_trip_time
                .with_label_values(&labels)
                .observe(rtt.as_secs_f64());
        }
        if let Some(ipdv) = ipdv {
            self.packet_delay_variation
                .with_label_values(&labels)
                .observe(ipdv);
        }
        self.last_observed_seq
            .with_label_values(&labels)
            .set(ping.seq.try_into().unwrap());
    }

    pub fn summary(&self, summary: SentReceivedSummary<&str>) {
        let labels = summary.labels();

        self.ping_sent
            .with_label_values(&labels)
            .inc_by(summary.sent.into());
        self.ping_received
            .with_label_values(&labels)
            .inc_by(summary.received.into());
    }

    pub fn error(&self, control: Control<&str>) {
        match control {
            Control::FpingError { target, .. } => {
                self.ping_errors.with_label_values(&[target, "fping"]).inc();
            }
            Control::IcmpError { target, .. } => {
                self.ping_errors.with_label_values(&[target, "icmp"]).inc();
            }
            _ => {}
        }
    }
}

impl Collector for PingMetrics {
    fn desc(&self) -> Vec<&prometheus::core::Desc> {
        vec![
            self.round_trip_time.desc(),
            self.packet_delay_variation.desc(),
            self.ping_sent.desc(),
            self.ping_received.desc(),
            self.ping_errors.desc(),
            self.last_observed_seq.desc(),
        ]
        .concat()
    }

    fn collect(&self) -> Vec<prometheus::proto::MetricFamily> {
        vec![
            self.round_trip_time.collect(),
            self.packet_delay_variation.collect(),
            self.ping_sent.collect(),
            self.ping_received.collect(),
            self.ping_errors.collect(),
            self.last_observed_seq.collect(),
        ]
        .concat()
    }
}
