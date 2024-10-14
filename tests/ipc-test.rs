#[cfg(test)]
mod tests {
    use log::{debug, info};
    use tfc::ipc::*;
    use tfc::logger;
    use tfc::progbase;

    fn setup_dirs() {
        // let current_dir = std::env::current_dir().expect("Failed to get current directory");
        // let current_dir_str = current_dir
        //     .to_str()
        //     .expect("Failed to convert path to string");
        // std::env::set_var("RUNTIME_DIRECTORY", current_dir_str);
        // std::env::set_var("CONFIGURATION_DIRECTORY", current_dir_str);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_async_send_recv_many() -> Result<(), Box<dyn std::error::Error>> {
        setup_dirs();
        let _ = progbase::try_init();
        // let _ = logger::init_combined_logger();

        let mut slot = SlotImpl::<i64>::new(Base::new("slot5", None));
        let mut signal = Signal::<i64>::new(Base::new("signal5", None));
        signal.init_task().await?;

        slot.async_connect(&signal.full_name())
            .await
            .expect("This should connect");

        let count = 100000;
        let rx_count = count.clone();
        let send_task = tokio::spawn(async move {
            for i in 1..count {
                signal.async_send(i).await.expect("This should not fail");
                tokio::task::yield_now().await;
            }
        });

        let recv_task = tokio::spawn(async move {
            for i in 1..rx_count {
                let recv_val = slot.recv().await.expect("This should not fail");
                assert_eq!(recv_val, i);
            }
        });

        let (recv_res, send_res) = tokio::join!(recv_task, send_task);

        send_res.expect("Sender task panicked");
        recv_res.expect("Receiver task panicked");
        Ok(())
    }

    // multithread
    #[tokio::test]
    async fn test_public_async_send_recv() -> Result<(), Box<dyn std::error::Error>> {
        setup_dirs();
        let _ = progbase::try_init();
        // let _ = logger::init_test_logger();
        let bus = zbus::connection::Builder::system()?
            .name(format!(
                "is.centroid.{}.{}",
                "test_public_async_send_recv", "test_public_async_send_recv"
            ))?
            .build()
            .await?;

        let mut slot = Slot::<bool>::new(bus.clone(), Base::new("slot6", None));
        let mut signal = Signal::<bool>::new(Base::new("signal6", None));
        log::info!("Waiting for init");
        signal.init_task().await?;
        log::info!("Init complete");

        log::info!("Connecting");
        slot.async_connect(&signal.full_name())
            .await
            .expect("This should connect");
        log::info!("Connected");
        let send_values = vec![
            true, true, false, false, true, true, false, false, true, true, false, false, true,
            true, false, false,
        ];
        let recv_values = vec![true, false, true, false, true, false, true, false];

        let recv_task = tokio::spawn(async move {
            let mut counter = 0;
            let mut watcher = slot.subscribe();
            for val in recv_values {
                watcher.changed().await.expect("This should not fail");
                let recv_val = watcher.borrow_and_update();
                counter += 1;
                info!("counter: {}", counter);
                if let Some(recv_val) = *recv_val {
                    info!("recv_val: {:?}, val: {}", recv_val, val);
                    assert_eq!(recv_val, val);
                } else {
                    assert!(false, "Something wrong happened");
                }
            }
        });

        let send_task = tokio::spawn(async move {
            // This sleep makes sure that the receiver has already started and is listening before sending
            // tokio::time::sleep(tokio::time::Duration::from_nanos(1)).await;
            for val in send_values {
                signal.async_send(val).await.expect("This should not fail");
                tokio::task::yield_now().await;
            }
        });

        let (recv_res, send_res) = tokio::join!(recv_task, send_task);

        send_res.expect("Sender task panicked");
        recv_res.expect("Receiver task panicked");
        Ok(())
    }

    #[tokio::test]
    async fn test_public_async_send_recv_many() -> Result<(), Box<dyn std::error::Error>> {
        setup_dirs();
        let _ = progbase::try_init();
        // let _ = logger::init_test_logger();
        let bus = zbus::connection::Builder::system()?
            .name(format!("is.centroid.{}.{}", "tfctest2", "tfctest2"))?
            .build()
            .await?;

        let mut slot = Slot::<bool>::new(bus.clone(), Base::new("slot7", None));
        let mut signal = Signal::<bool>::new(Base::new("signal7", None));
        log::info!("Waiting for init");
        signal.init_task().await?;
        log::info!("Init complete");

        log::info!("Connecting");
        slot.async_connect(&signal.full_name())
            .await
            .expect("This should connect");

        log::info!("Connected");

        let count = 100000;
        let rx_count = count.clone();
        let recv_task = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let mut watcher = slot.subscribe();
            debug!("Receiving!!!!!!!!!!!!!!!!!!!!!!!!");
            for val in 1..rx_count {
                watcher.changed().await.expect("This should not fail");
                let recv_val = watcher.borrow_and_update();
                info!("counter: {}", val);
                if let Some(recv_val) = *recv_val {
                    info!("recv_val: {:?}, val: {}", recv_val, val);
                    assert_eq!(recv_val, val % 2 == 0);
                } else {
                    assert!(false, "Something wrong happened");
                }
            }
            let duration = start.elapsed();
            println!("Duration: {:?}", duration);
        });

        let send_task = tokio::spawn(async move {
            debug!("Sending!!!!!!!!!!!!!!!!!!!!!!!!");
            for val in 1..count {
                signal
                    .async_send(val % 2 == 0)
                    .await
                    .expect("This should not fail");
                tokio::task::yield_now().await;
            }
        });

        let (recv_res, send_res) = tokio::join!(recv_task, send_task);

        send_res.expect("Sender task panicked");
        recv_res.expect("Receiver task panicked");
        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_public_recv_after_send() -> Result<(), Box<dyn std::error::Error>> {
        setup_dirs();
        let _ = progbase::try_init();
        // let _ = logger::init_combined_logger();

        let bus = zbus::connection::Builder::system()?
            .name(format!("is.centroid.{}.{}", "tfctest3", "tfctest3"))?
            .build()
            .await?;

        let mut slot = Slot::<bool>::new(bus.clone(), Base::new("slot8", None));
        let mut signal = Signal::<bool>::new(Base::new("signal8", None));
        signal.init_task().await?;

        slot.async_connect(&signal.full_name())
            .await
            .expect("This should connect");

        signal.async_send(true).await.expect("This should not fail");

        let recv =
            tokio::time::timeout(tokio::time::Duration::from_secs(1), slot.async_recv()).await;

        let mut value = match recv {
            Ok(inner_result) => match inner_result {
                Ok(value) => value,
                Err(e) => {
                    panic!("Error receiving value: {:?}", e);
                }
            },
            Err(_) => {
                panic!("Timeout occurred");
            }
        };

        assert_eq!(value.take().unwrap(), true);

        Ok(())
    }
    #[tokio::test]
    async fn initialize_button() {
        let bus = zbus::connection::Builder::system()
            .expect("Build a bus")
            .build()
            .await
            .expect("Build success");
        let mut slot = Slot::<bool>::new(bus.clone(), Base::new("test_slot", None));
        let mut signal = Signal::<bool>::new(Base::new("test_signal", None));

        // Connect them together
        slot.connect(&signal.full_name()).expect("Connect success");
        //tokio::time::sleep(Duration::from_millis(1000)).await;

        // Send some values and check they are correct
        // Send true value
        signal.async_send(true).await.expect("Send success");
        let value = slot
            .async_recv()
            .await
            .expect("Have some value")
            .take()
            .unwrap();
        assert_eq!(value, true);

        // Send false value
        // todo
        // signal.async_send(false).await.expect("Send success");
        // tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        // assert_eq!(slot.value().expect("Has value"), false);
    }

    #[tokio::test]
    async fn test_watch() {
        let (tx, mut rx) = tokio::sync::watch::channel(0);

        let mut rx2 = rx.clone();
        let mut rx3 = rx.clone();

        let recv_task1 = tokio::spawn(async move {
            let start = std::time::Instant::now();
            for i in 1..100000 {
                rx3.changed().await.expect("This should not fail");
                let val = rx3.borrow();
                assert_eq!(*val, i);
            }
            let duration = start.elapsed();
            println!("Duration1: {:?}", duration);
        });

        let recv_task2 = tokio::spawn(async move {
            let start = std::time::Instant::now();
            for i in 1..100000 {
                rx2.changed().await.expect("This should not fail");
                let val = rx2.borrow();
                assert_eq!(*val, i);
            }
            let duration = start.elapsed();
            println!("Duration2: {:?}", duration);
        });

        let recv_task3 = tokio::spawn(async move {
            let start = std::time::Instant::now();
            for i in 1..100000 {
                rx.changed().await.expect("This should not fail");
                let val = rx.borrow();
                assert_eq!(*val, i);
            }
            let duration = start.elapsed();
            println!("Duration3: {:?}", duration);
        });

        let send_task = tokio::spawn(async move {
            for i in 1..100000 {
                // tokio::time::sleep(tokio::time::Duration::from_nanos(1)).await;
                // for _ in 1..100000 {
                //     unsafe {
                //         std::arch::asm!("nop");
                //     }
                // }
                tx.send(i).expect("Send success");
                tokio::task::yield_now().await;
            }
        });

        let (recv_res1, recv_res2, recv_res3, send_res) =
            tokio::join!(recv_task1, recv_task2, recv_task3, send_task);

        recv_res1.expect("Receiver task panicked");
        recv_res2.expect("Receiver task panicked");
        recv_res3.expect("Receiver task panicked");
        send_res.expect("Sender task panicked");
    }
    // #[tokio::test]
    // async fn basic_send_recv_test() -> Result<(), Box<dyn std::error::Error>> {
    //     progbase::init();

    //     let _ = logger::init_combined_logger();

    //     let bus = zbus::connection::Builder::system()?
    //         .name(format!("is.centroid.{}.{}", "tfctest", "tfctest"))?
    //         .build()
    //         .await?;
    //     let mut my_little_slot = Slot::<bool>::new(bus.clone(), Base::new("my_little_slot", None));
    //     let mut my_little_signal = Signal::<bool>::new(bus, Base::new("my_little_signal", None));

    //     let vals_to_send = vec![
    //         false, false, true, true, false, false, true, true, false, false, true, true, false,
    //         false,
    //     ];
    //     let vals_to_recv = vec![false, true, false, true, false, true, false];

    //     my_little_slot
    //         .connect(my_little_signal.full_name())
    //         .expect("This should connect");

    //     let mut stream = my_little_slot.stream();
    //     let recv_task = tokio::spawn(async move {
    //         let mut counter = 0;
    //         for expected_val in vals_to_recv {
    //             tokio::select! {
    //                 val = stream.next() => {
    //                     assert_eq!(val.expect("This should not fail"), expected_val);
    //                     counter += 1;
    //                 },
    //                 _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
    //                     panic!("No value received for 1 second, only received {} values", counter);
    //                 }
    //             }
    //         }
    //     });

    //     let send_task = tokio::spawn(async move {
    //         for val in vals_to_send {
    //             my_little_signal
    //                 .async_send(val)
    //                 .await
    //                 .expect("This should not fail");
    //         }
    //     });

    //     let (recv_res, send_res) = tokio::join!(recv_task, send_task);

    //     recv_res.expect("Receiver task panicked");

    //     send_res.expect("Sender task panicked");

    //     Ok(())
    // }
}
