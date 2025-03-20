use ringbuf::traits::{Consumer, Observer, Producer};
use ringbuf::HeapRb;

/// A median filter for real numbers that uses a ring buffer for the window
/// and maintains a sorted vector for fast median lookup.
pub struct MedianFilter {
    window: HeapRb<f64>,
    sorted: Vec<f64>,
}

impl MedianFilter {
    /// Creates a new MedianFilter with the given window capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            window: HeapRb::new(capacity),
            sorted: Vec::with_capacity(capacity),
        }
    }

    /// Consumes a new value, updating the ring buffer and the sorted vector,
    /// then returns the current median.
    pub fn consume(&mut self, value: f64) -> f64 {
        // If the ring buffer is full, remove the oldest element.
        if self.window.is_full() {
            let old_value = self
                .window
                .try_pop()
                .expect("Buffer is full, but pop failed unexpectedly");
            // Remove the old value from the sorted vector.
            // Use binary search to find its index.
            if let Ok(pos) = self
                .sorted
                .binary_search_by(|x| x.partial_cmp(&old_value).unwrap())
            {
                self.sorted.remove(pos);
            } else {
                panic!("Old value not found in sorted vector");
            }
        }
        // Insert the new value into the ring buffer.
        self.window
            .try_push(value)
            .expect("Push should succeed after removal");

        // Insert the new value in the sorted vector in its correct position.
        let pos = self
            .sorted
            .binary_search_by(|x| x.partial_cmp(&value).unwrap())
            .unwrap_or_else(|e| e);
        self.sorted.insert(pos, value);

        self.median()
    }

    /// Computes and returns the current median from the sorted vector.
    pub fn median(&self) -> f64 {
        let n = self.sorted.len();
        if n == 0 {
            return 0.0;
        }
        if n % 2 == 1 {
            self.sorted[n / 2]
        } else {
            (self.sorted[n / 2 - 1] + self.sorted[n / 2]) / 2.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_median_increasing_sequence() {
        // Create a median filter with a window size of 5.
        let mut mf = MedianFilter::new(5);

        // Consume values and test median.
        assert_eq!(mf.consume(1.0), 1.0); // Window: [1]
        assert_eq!(mf.consume(2.0), 1.5); // Window: [1,2]
        assert_eq!(mf.consume(3.0), 2.0); // Window: [1,2,3]
        assert_eq!(mf.consume(4.0), 2.5); // Window: [1,2,3,4]
        assert_eq!(mf.consume(5.0), 3.0); // Window: [1,2,3,4,5] -> median 3

        // Adding a new value causes the oldest (1.0) to be removed.
        // Window becomes: [2,3,4,5,6] with median 4.
        assert_eq!(mf.consume(6.0), 4.0);
    }

    #[test]
    fn test_median_random_sequence() {
        // Create a median filter with a window size of 3.
        let mut mf = MedianFilter::new(3);

        assert_eq!(mf.consume(10.0), 10.0); // Window: [10]
        assert_eq!(mf.consume(1.0), 5.5); // Window: [10,1] -> median (1+10)/2 = 5.5
        assert_eq!(mf.consume(5.0), 5.0); // Window: [10,1,5] -> sorted: [1,5,10] -> median 5

        // Next, window slides: remove 10.0, window becomes: [1,5,3]
        assert_eq!(mf.consume(3.0), 3.0); // Sorted window: [1,3,5] -> median 3
    }
}
