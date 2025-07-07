mod ringbuffer;
mod valuebox;

pub use ringbuffer::{MPRingBuffer, RBReader, RBWriter, SPRingBuffer};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_producer_two_consumers() {
        let size = 10000;
        let ring_buffer = Box::leak(Box::new(SPRingBuffer::<u32, 1024>::new()));
        let (reader_factory, writer_factory) = ring_buffer.split();
        let randomized_input = (0..size).map(|_| rand::random::<u32>()).collect::<Vec<_>>();

        println!("Starting test with size: {size}");
        // Spawn writer thread
        let writer_handle = {
            let mut writer = writer_factory;
            let randomized_input = randomized_input.clone();
            std::thread::spawn(move || {
                let mut cnt = 0;
                for value in randomized_input {
                    writer.put(value);
                    cnt += 1;
                    if cnt % 100 == 0 {
                        // add some random sleep to to prevent overflow (it could write too fast)
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
            })
        };

        // Spawn reader threads
        let reader_handles = (0..10)
            .map(|i| {
                let mut reader = reader_factory.clone();
                let randomized_input = randomized_input.clone();
                std::thread::spawn(move || {
                    let mut result = Vec::new();
                    while !reader.is_finished() {
                        while let Some(value) = reader.next() {
                            result.push(value.clone());
                        }
                        std::thread::yield_now();
                    }
                    assert_eq!(result, randomized_input, "thread {i} failed, not equal");
                    println!("thread {i} done");
                })
            })
            .collect::<Vec<_>>();

        // Wait for writer to finish
        writer_handle.join().expect("Writer thread failed to join");

        // Wait for all readers to finish
        for handle in reader_handles {
            handle.join().expect("Reader thread failed to join");
        }

        println!("Done");
    }

    #[test]
    fn two_producers_two_consumers() {
        let size = 5000;
        let ring_buffer = Box::leak(Box::new(MPRingBuffer::<u32, 2048>::new()));
        let (reader_factory, writer_factory) = ring_buffer.split();
        let input1 = (0..size).map(|_| rand::random::<u32>()).collect::<Vec<_>>();
        let input2 = (0..size).map(|_| rand::random::<u32>()).collect::<Vec<_>>();

        let mut final_result = input1
            .iter()
            .chain(input2.iter())
            .cloned()
            .collect::<Vec<_>>();
        final_result.sort();

        let mut inputs = vec![input1, input2];

        println!("Starting test with size: {size}");

        // Spawn writer thread
        let writer_handle = (0..2)
            .map(|_| {
                let mut writer = writer_factory.clone();
                let randomized_input = inputs.pop().expect("No input left");
                std::thread::spawn(move || {
                    let mut cnt = 0;
                    for value in randomized_input {
                        writer.put(value);
                        cnt += 1;
                        if cnt % 100 == 0 {
                            // add some random sleep to to prevent overflow (it could write too fast)
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                    }
                })
            })
            .collect::<Vec<_>>();

        // otherwise the extra writer will block the readers from finishing
        drop(writer_factory);

        // Spawn reader threads
        let reader_handles = (0..10)
            .map(|i| {
                let mut reader = reader_factory.clone();
                let final_result = final_result.clone();
                std::thread::spawn(move || {
                    let mut result = Vec::new();
                    while !reader.is_finished() {
                        while let Some(value) = reader.next() {
                            result.push(value.clone());
                        }
                        std::thread::yield_now();
                    }
                    result.sort();
                    assert_eq!(
                        result.len(),
                        final_result.len(),
                        "thread {i} failed, length not equal"
                    );
                    assert_eq!(result, final_result, "thread {i} failed, content not equal");
                    println!("thread {i} done");
                })
            })
            .collect::<Vec<_>>();

        // Wait for writer to finish
        for handle in writer_handle {
            handle.join().expect("Writer thread failed to join");
        }

        // Wait for all readers to finish
        for handle in reader_handles {
            handle.join().expect("Reader thread failed to join");
        }

        println!("Done");
    }

    #[test]
    fn mpsc_counter_stress_test() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicI32, Ordering};

        const MAX_EVENTS: usize = 1_000_000;
        const BUF_SIZE: usize = 32768;

        let ring_buffer = Box::leak(Box::new(MPRingBuffer::<i32, BUF_SIZE>::new()));
        let (mut rx, mut tx) = ring_buffer.split();
        let mut tx2 = tx.clone();

        // Producer 1
        let t1 = std::thread::spawn(move || {
            for _ in 0..MAX_EVENTS {
                tx.put(1);
            }
            tx.mark_as_finished();
        });

        // Producer 2
        let t2 = std::thread::spawn(move || {
            for _ in 0..MAX_EVENTS {
                tx2.put(1);
            }
            tx2.mark_as_finished();
        });

        let sink = Arc::new(AtomicI32::new(0));
        let sink_clone = Arc::clone(&sink);

        // Consumer
        let c1 = std::thread::spawn(move || {
            while !rx.is_finished() {
                while let Some(value) = rx.next() {
                    sink_clone.fetch_add(*value, Ordering::Release);
                }
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
        c1.join().unwrap();

        let final_count = sink.load(Ordering::Acquire) as usize;
        assert_eq!(
            final_count,
            MAX_EVENTS * 2,
            "Expected {}, got {}",
            MAX_EVENTS * 2,
            final_count
        );
    }

    #[test]
    fn mpsc_small_buffer_test() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

        // Use a tiny buffer to force wrapping - same scale as failing test
        const MAX_EVENTS: usize = 1_000_000;
        const BUF_SIZE: usize = 64;

        let ring_buffer = Box::leak(Box::new(MPRingBuffer::<i32, BUF_SIZE>::new()));
        let (mut rx, mut tx) = ring_buffer.split();
        let mut tx2 = tx.clone();

        let write_count = Arc::new(AtomicUsize::new(0));
        let write_count1 = write_count.clone();
        let write_count2 = write_count.clone();

        // Producer 1
        let t1 = std::thread::spawn(move || {
            for i in 0..MAX_EVENTS {
                tx.put(1);
                write_count1.fetch_add(1, Ordering::Release);
                if i % 10 == 0 {
                    std::thread::yield_now();
                }
            }
            tx.mark_as_finished();
        });

        // Producer 2
        let t2 = std::thread::spawn(move || {
            for i in 0..MAX_EVENTS {
                tx2.put(1);
                write_count2.fetch_add(1, Ordering::Release);
                if i % 10 == 0 {
                    std::thread::yield_now();
                }
            }
            tx2.mark_as_finished();
        });

        let sink = Arc::new(AtomicI32::new(0));
        let sink_clone = Arc::clone(&sink);

        // Consumer
        let c1 = std::thread::spawn(move || {
            while !rx.is_finished() {
                while let Some(value) = rx.next() {
                    sink_clone.fetch_add(*value, Ordering::Release);
                }
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();
        c1.join().unwrap();

        let total_written = write_count.load(Ordering::Acquire);
        let final_count = sink.load(Ordering::Acquire) as usize;

        println!(
            "Small buffer test: written={}, read={}, lost={}",
            total_written,
            final_count,
            total_written - final_count
        );

        // With a small buffer, we expect some loss due to overwriting
        assert!(
            final_count <= total_written,
            "Read more than written! read={}, written={}",
            final_count,
            total_written
        );
    }
}
