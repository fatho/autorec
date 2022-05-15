use color_eyre::Result;
use futures_util::future::ready;
use futures_util::stream::StreamExt;
use tokio_udev::{AsyncMonitorSocket, MonitorBuilder};
use tokio_util::task::LocalPoolHandle;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    // Local thread pool for udev because those types are !Send
    let pool = LocalPoolHandle::new(1);

    let udev_thread = pool.spawn_pinned(|| async move {
        info!("Initializing udev");
        let builder = MonitorBuilder::new()?;
        // .match_subsystem_devtype("usb", "usb_device")?;

        let monitor: AsyncMonitorSocket = builder.listen()?.try_into()?;

        info!("Waiting for events");

        monitor
            .for_each(|event| {
                match event {
                    Ok(event) => {
                        info!(
                            "Hotplug event: {}: {}",
                            event.event_type(),
                            event.device().syspath().display()
                        );
                    },
                    Err(err) => error!("Failed to get udev event {}", err),
                }
                ready(())
            })
            .await;

        Result::<()>::Ok(())
    });

    let exit_signal = tokio::signal::ctrl_c();

    tokio::select! {
        _ = exit_signal => {
            info!("Terminated");
            Ok(())
        },
        thread_result = udev_thread => {
            info!("Udev thread exited");
            match thread_result {
                Ok(udev_result) => {
                    error!("Udev failed");
                    udev_result
                },
                Err(join_err) => {
                    error!("Failed to join udev thread: {}", join_err);
                    Err(join_err.into())
                },
            }
        }
    }
}
