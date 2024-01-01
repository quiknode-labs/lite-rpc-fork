use self::tc1::Tc1;
use self::tc2::Tc2;
use self::tc3::Tc3;
use self::tc4::Tc4;

pub mod tc1;
pub mod tc2;
pub mod tc3;
pub mod tc4;

#[async_trait::async_trait]
pub trait Strategy {
    type Output: serde::Serialize;

    async fn execute(&self) -> anyhow::Result<Self::Output>;
}

#[derive(clap::Subcommand, Debug)]
pub enum Strategies {
    Tc1(Tc1),
    Tc2(Tc2),
    Tc3(Tc3),
    Tc4(Tc4),
}

impl Strategies {
    pub async fn execute(&self, metrics_file_name: &str) -> anyhow::Result<()> {
        let mut csv_writer = csv::Writer::from_path(metrics_file_name).unwrap();

        match self {
            Strategies::Tc1(tc1) => csv_writer.serialize(tc1.execute().await?)?,
            Strategies::Tc2(tc2) => csv_writer.serialize(tc2.execute().await?)?,
            Strategies::Tc3(tc3) => csv_writer.serialize(tc3.execute().await?)?,
            Strategies::Tc4(tc4) => csv_writer.serialize(tc4.execute().await?)?,
        }

        csv_writer.flush()?;

        Ok(())
    }
}
