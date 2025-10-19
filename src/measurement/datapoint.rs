
use crate::core::DataPoint;
use crate::measurement::Measure;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
impl Measure for DataPoint {
    type Data = DataPoint;

    async fn measure(&mut self) -> Result<DataPoint> {
        Ok(self.clone())
    }

    async fn data_stream(&self) -> Result<mpsc::Receiver<DataPoint>> {
        // This is a bit of a hack, but it will work for now.
        let (sender, mut receiver) = mpsc::channel(1);
        sender.send(self.clone()).await.ok();
        Ok(receiver)
    }
}
