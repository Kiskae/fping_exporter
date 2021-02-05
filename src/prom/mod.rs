mod metrics;

pub use metrics::PingMetrics;
use prometheus::core::{Collector, Desc};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub struct LockedCollector<C>(Vec<Desc>, Arc<Mutex<C>>);

impl<C: Collector> From<Arc<Mutex<C>>> for LockedCollector<C> {
    fn from(collector: Arc<Mutex<C>>) -> Self {
        let descs = collector
            .lock()
            .unwrap()
            .desc()
            .into_iter()
            .cloned()
            .collect();
        Self(descs, collector)
    }
}

impl<C: Collector> Collector for LockedCollector<C> {
    fn desc(&self) -> Vec<&prometheus::core::Desc> {
        self.0.iter().collect()
    }

    fn collect(&self) -> Vec<prometheus::proto::MetricFamily> {
        self.1.lock().unwrap().collect()
    }
}
