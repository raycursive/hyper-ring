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
                    let mut cursor = 0;
                    let mut result = Vec::new();
                    while cursor < size {
                        while let Some(value) = reader.next() {
                            cursor += 1;
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
        let size = 10000;
        let ring_buffer = Box::leak(Box::new(MPRingBuffer::<u32, 1024>::new()));
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

        // Spawn reader threads
        let reader_handles = (0..10)
            .map(|i| {
                let mut reader = reader_factory.clone();
                let final_result = final_result.clone();
                std::thread::spawn(move || {
                    let mut cursor = 0;
                    let mut result = Vec::new();
                    while cursor < size * 2 {
                        while let Some(value) = reader.next() {
                            cursor += 1;
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
}
