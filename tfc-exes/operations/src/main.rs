use tfc::logger;
use tfc::progbase;
use tokio;

mod operations;

use crate::operations::server::OperationsImpl;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    progbase::init();
    logger::init_combined_logger()?;
    let formatted_name = format!(
        "is.centroid.{}.{}",
        progbase::exe_name(),
        progbase::proc_name()
    );
    let bus = zbus::connection::Builder::system()?
        .name(formatted_name)?
        .build()
        .await?;

    let _ = OperationsImpl::spawn(bus.clone());

    std::future::pending::<()>().await;
    Ok(())
}
