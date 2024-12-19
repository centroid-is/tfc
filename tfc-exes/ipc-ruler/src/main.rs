use tfc::logger;
use tfc::progbase;

mod lib;
use crate::lib::IpcRuler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    progbase::init();
    logger::init_combined_logger()?;
    let bus = zbus::connection::Builder::system()?
        .name(crate::lib::DBUS_SERVICE)?
        .build()
        .await?;

    let _ = IpcRuler::spawn(bus.clone(), false);

    std::future::pending::<()>().await;
    Ok(())
}
