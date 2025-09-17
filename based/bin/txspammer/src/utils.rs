pub struct Percentile {
    pub data: Vec<f64>,
    pub sorted: bool,
}

impl Percentile {
    pub fn new() -> Self {
        Self { data: Vec::new(), sorted: false }
    }

    pub fn add(&mut self, value: f64) {
        self.data.push(value);
        self.sorted = false;
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn percentile(&mut self, p: f64) -> Option<f64> {
        if self.data.is_empty() {
            return None;
        }
        if !self.sorted {
            self.data.sort_by(|a, b| a.partial_cmp(b).unwrap());
            self.sorted = true;
        }
        let rank = (p / 100.0) * (self.data.len() - 1) as f64;
        let lower_index = rank.floor() as usize;
        let upper_index = rank.ceil() as usize;
        if lower_index == upper_index {
            Some(self.data[lower_index])
        } else {
            let lower_value = self.data[lower_index];
            let upper_value = self.data[upper_index];
            Some(lower_value + (upper_value - lower_value) * (rank - lower_index as f64))
        }
    }
}
