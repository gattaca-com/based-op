use std::{
    fmt::{Debug, Display},
    marker::PhantomData,
};

use bop_common::{
    communication::Consumer,
    time::{Duration, Instant, Nanos},
};

use crate::{circular_buffer::CircularBuffer, tui::RenderFlags};

pub trait Statisticable: Into<u64> + From<u64> + Display + Clone + Copy + PartialEq {
    fn to_plotpoint(&self) -> f64 {
        Into::<u64>::into(*self) as f64
    }
}

impl Statisticable for Duration {}

impl Statisticable for Nanos {}

impl Statisticable for MsgPer10Sec {}

#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct MsgPer10Sec(pub u64);

impl Display for MsgPer10Sec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:.1}", self.0 as f64 / 10.0)
    }
}

impl From<MsgPer10Sec> for u64 {
    fn from(value: MsgPer10Sec) -> Self {
        value.0
    }
}
impl From<u64> for MsgPer10Sec {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// Keep track of msg latencies
/// All in nanos
#[derive(Debug, Clone, Copy)]
pub struct DataPoint<T: Statisticable> {
    pub avg: u64,
    pub min: u64,
    pub max: u64,
    pub median: u64,
    pub n_samples: usize,
    pub vline: bool,
    pub rate: MsgPer10Sec,
    _p: std::marker::PhantomData<T>,
}

impl<T: Statisticable> DataPoint<T> {
    fn block_start() -> DataPoint<T> {
        Self { vline: true, ..Default::default() }
    }
}

impl<T: Statisticable> Default for DataPoint<T> {
    fn default() -> Self {
        Self {
            avg: 0,
            min: u64::MAX,
            max: 0,
            median: 0,
            n_samples: 0,
            vline: false,
            rate: Default::default(),
            _p: PhantomData {},
        }
    }
}

#[derive(Debug, Clone)]
pub struct Statistics<T: Statisticable> {
    pub title: String,
    // These are updated when Start
    measurements: CircularBuffer<u64>,
    pub datapoints: CircularBuffer<DataPoint<T>>,

    // Running values, to go into the next datapoint
    avg: u64,
    min: u64,
    max: u64,
    samples: usize,

    // tot_count does not necessarily need to be equal to the sum of the samples
    // of the datapoints. If a timer is spamming messages, the timekeeper
    // won't be able to register all of them as samples, yet the
    // count of the timer queue will reflect the amount of messages
    // that were processed and is what is reflected here.
    // Using the delta from the queue count with this value + the
    // elapsed since last_t, we can know what the msgs/s was even
    // if we're not registering each of them as a sample
    tot_count: usize,

    last_t: Instant,

    // This will be subtracted from each measurement, useful for e.g. clock overhead
    offset: u64,
    samples_per_median: usize,
    pub flags: RenderFlags,
    got_one: bool,
}

impl<T: Statisticable> Statistics<T> {
    pub fn new(title: String, samples_per_median: usize, n_datapoints: usize, offset: T) -> Self {
        let measurements = CircularBuffer::new(samples_per_median);

        Self {
            title,
            measurements,
            datapoints: CircularBuffer::new(n_datapoints),
            min: u64::MAX,
            max: 0,
            avg: 0,
            offset: offset.into(),
            samples_per_median,
            samples: 0,
            tot_count: 0,
            flags: RenderFlags::ShowAverages,
            last_t: Instant::now(),
            got_one: false,
        }
    }

    fn corrected_or_zero(&self, t: u64) -> u64 {
        t.saturating_sub(self.offset)
    }

    pub fn register_datapoint(&mut self, mut tot_count: usize, block_start: bool) {
        if self.measurements.is_empty() {
            if self.datapoints.is_empty() {
                return;
            }
            let d = if block_start { DataPoint::block_start() } else { DataPoint::default() };

            self.datapoints.push(d);
            return;
        }
        if tot_count == 0 {
            tot_count = self.samples;
        }
        let median = self.measurements.median();

        let count_delta = tot_count.saturating_sub(self.tot_count) as u64;
        let elapsed_since_last = self.last_t.elapsed().0;
        let rate = if count_delta > elapsed_since_last * 1000 {
            MsgPer10Sec(Duration::from_secs(10).0.saturating_mul(count_delta / elapsed_since_last))
        } else {
            MsgPer10Sec(Duration::from_secs(10).0.saturating_mul(count_delta) / elapsed_since_last)
        };

        self.datapoints.push(DataPoint {
            avg: self.corrected_or_zero(self.avg),
            min: self.corrected_or_zero(self.min),
            max: self.corrected_or_zero(self.max),
            median: self.corrected_or_zero(median),
            n_samples: self.samples,
            vline: block_start,
            rate,
            ..Default::default()
        });
        self.tot_count = tot_count;
        self.reset();
    }

    fn reset(&mut self) {
        self.measurements.clear();
        self.min = u64::MAX;
        self.max = 0;
        self.samples = 0;
        self.got_one = true;
        self.last_t = Instant::now();
    }

    // returns true if full datapoint is captured
    pub fn track(&mut self, el: T) {
        let el = el.into();

        if el > self.max {
            self.max = el;
        }

        if el < self.min {
            self.min = el;
        }

        let avg = self.avg * self.samples as u64;

        self.samples += 1;
        self.avg = (avg + el) / self.samples as u64;

        self.measurements.push(el);
    }

    pub fn tot_samples(&self) -> usize {
        self.datapoints.iter().map(|d| d.n_samples).sum()
    }

    pub fn is_empty(&self) -> bool {
        !self.got_one
    }

    pub fn handle_messages<M: 'static + Copy + Default + Into<T>>(&mut self, consumer: &mut Consumer<M>) {
        let mut captured_one = false;
        let mut done = false;
        let mut c = 0;
        // loops until either a full datapoint was captured or no messages are pending
        while !done {
            done |= !consumer.consume(|&mut msg| {
                captured_one = true;
                self.track(msg.into());
                c += 1;
                done = c == self.samples_per_median;
            });
        }
    }

    pub fn toggle(&mut self, flags: RenderFlags) {
        self.flags ^= flags
    }
}
