use std::fmt::{Debug, Display};

use ratatui::{
    prelude::*,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};
use symbols::Marker;

use crate::statistics::{DataPoint, MsgPer10Sec, Statisticable, Statistics};

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Ord, Eq, Default)]
    pub struct RenderFlags: u8 {
        const ShowMin      = 0b00000001;
        const ShowMax      = 0b00000010;
        const ShowMedian   = 0b00000100;
        const ShowAverages = 0b0001000;
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlotSeries<T> {
    color: Color,
    label: String,
    graph_type: GraphType,
    marker: Marker,
    data: Vec<(f64, f64)>,
    y_min: u64,
    y_max: u64,
    last_val: u64,
    _p: std::marker::PhantomData<T>,
}
impl<T: Display + Into<u64> + Default + Clone> PlotSeries<T> {
    pub fn color(self, color: Color) -> Self {
        Self { color, ..self }
    }

    pub fn label<S: ToString>(self, label: S) -> Self {
        Self { label: label.to_string(), ..self }
    }

    pub fn graph_type(self, graph_type: GraphType) -> Self {
        Self { graph_type, ..self }
    }

    pub fn marker(self, marker: Marker) -> Self {
        Self { marker, ..self }
    }

    pub fn data(self, data: impl Iterator<Item = (usize, u64)>) -> Self {
        let mut y_min = u64::MAX;
        let mut y_max = u64::MIN;
        let mut last_val = Default::default();
        let data = data
            .map(|(i, d)| {
                last_val = d;
                y_min = y_min.min(d);
                y_max = y_max.max(d);
                (i as f64, d as f64)
            })
            .collect();

        Self { data, y_min, y_max, last_val, ..self }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[allow(clippy::wrong_self_convention)]
    fn to_dataset(&mut self, y_min: f64, y_max: f64) -> Dataset {
        if self.len() == 2 && self.data[0].1 == 0.0 && self.data[1].1 == 0.0 {
            self.data[0].1 = y_min;
            self.data[1].1 = y_max;
        }
        let mut o = Dataset::default().marker(self.marker).style(Style::new().fg(self.color)).graph_type(self.graph_type).data(&self.data);
        if !self.label.is_empty() {
            o = o.name(self.label.as_str());
        }
        o
    }
}

#[derive(Debug, Clone)]
pub struct StatsPlot<T> {
    title: String,
    y_min: u64,
    y_max: u64,
    plot_series: Vec<PlotSeries<T>>,
}

impl<T: Statisticable + Default> StatsPlot<T> {
    pub fn new<S: ToString>(title: S) -> Self {
        Self { title: title.to_string(), y_min: u64::MAX, y_max: 0, plot_series: Default::default() }
    }
    pub fn push(&mut self, series: PlotSeries<T>) {
        if !(series.y_min == 0 && series.y_max == 0) {
            self.y_min = self.y_min.min(series.y_min);
            self.y_max = self.y_max.max(series.y_max);
        }
        self.plot_series.push(series);
    }

    pub fn render(&mut self, frame: &mut Frame, rect: Rect, title: Option<String>, x_max: Option<usize>) {
        let mut last_point_str = if self.title.is_empty() { "Last point: ".to_string() } else { format!("{}: Last point: ", self.title) };
        let mut got_one = false;
        for t in self.plot_series.iter().filter(|t| t.last_val != 0) {
            got_one = true;
            last_point_str.push_str(&format!("{}: {} - ", t.label, T::from(t.last_val)))
        }
        let text: Line = if got_one { last_point_str[0..last_point_str.len() - 3].into() } else { last_point_str.into() };

        let mut block = Block::new().borders(Borders::ALL);
        if let Some(title) = title {
            block = block.title(title);
        }
        let area = block.inner(rect);
        let sub_layout = Layout::new(Direction::Vertical, [Constraint::Length(1), Constraint::Fill(1)]).split(area);
        frame.render_widget(block, rect);

        let y_min = self.y_min.min(self.y_max);
        let y_max = self.y_max;

        let ylabels = vec![T::from(y_min).to_string(), format!("{}", T::from((y_min + y_max) / 2_u64)), T::from(y_max).to_string()];

        let x_max = x_max.unwrap_or_else(|| self.plot_series.iter().flat_map(|s| s.data.iter().map(|(i, _)| *i as usize)).max().unwrap_or_default());

        let xlabels = vec!["0".to_string(), x_max.to_string()];

        let xaxis = Axis::default().bounds([0.0, x_max as f64]).style(Style::default().fg(Color::LightBlue)).labels(xlabels);
        let y_min = 0.95 * (y_min as f64);
        let y_max = 1.05 * (y_max as f64);
        let datasets = self.plot_series.iter_mut().map(|d| d.to_dataset(y_min, y_max)).collect();
        let yaxis = Axis::default().bounds([y_min, y_max]).style(Style::default().fg(Color::LightBlue)).labels(ylabels);

        let chart = Chart::new(datasets).x_axis(xaxis).y_axis(yaxis);
        frame.render_widget(text, sub_layout[0]);
        frame.render_widget(chart, sub_layout[1]);
    }
}

impl<T: Statisticable> Statistics<T> {
    #[allow(clippy::filter_map_bool_then)]
    pub fn add_block_starts<D: Default + Statisticable>(&self, plot: &mut StatsPlot<D>) {
        let block_starts = self.datapoints.iter().enumerate().filter_map(move |(i, d)| d.vline.then(|| vec![(i, 0), (i, 0)]));
        for d in block_starts {
            plot.push(PlotSeries::default().color(Color::Red).marker(Marker::Braille).graph_type(GraphType::Line).data(d.into_iter()));
        }
    }

    pub fn to_plot_data<'a, F: Fn(&DataPoint<T>) -> u64 + 'a>(&'a self, f: F) -> impl Iterator<Item = (usize, u64)> + 'a {
        self.datapoints.iter().enumerate().filter(|(_, t)| t.n_samples != 0).map(move |(i, t)| (i, f(t)))
    }

    pub fn report(&self, name: &str, frame: &mut Frame, rect: Rect)
    where
        T: Default,
    {
        let mut avg = 0;
        let mut tot = 0;
        for d in self.datapoints.iter().filter(|d| d.n_samples != 0) {
            avg += d.avg;
            tot += 1;
        }
        if tot != 0 {
            avg /= tot;
        }

        let title = if name.is_empty() { self.title.to_string() } else { format!("{} report for {name}", self.title) };
        let mut plot = StatsPlot::<T>::new(title);

        if self.flags.contains(RenderFlags::ShowAverages) {
            plot.push(PlotSeries::default().label("avg").graph_type(GraphType::Line).data(self.to_plot_data(|t| t.avg)));
        }

        if self.flags.contains(RenderFlags::ShowMin) {
            plot.push(PlotSeries::default().label("min").color(Color::Green).graph_type(GraphType::Line).data(self.to_plot_data(|t| t.min)));
        }

        if self.flags.contains(RenderFlags::ShowMax) {
            plot.push(PlotSeries::default().label("max").color(Color::Red).graph_type(GraphType::Line).data(self.to_plot_data(|t| t.max)));
        }

        if self.flags.contains(RenderFlags::ShowMedian) {
            plot.push(PlotSeries::default().label("med").color(Color::Yellow).graph_type(GraphType::Line).data(self.to_plot_data(|t| t.median)));
        }

        self.add_block_starts(&mut plot);
        plot.render(frame, rect, Some(format!("Running avg: {}", T::from(avg))), Some(self.datapoints.len()));
    }

    pub fn report_msg_per_sec(&self, frame: &mut Frame, rect: Rect) {
        let mut plot = StatsPlot::<MsgPer10Sec>::new("");

        plot.push(
            PlotSeries::default()
                .label("msg/s")
                .graph_type(GraphType::Line)
                .data(self.datapoints.iter().enumerate().map(|(i, t)| (i, t.rate.into()))),
        );
        self.add_block_starts(&mut plot);
        plot.render(frame, rect, Some("Msg/s".to_string()), Some(self.datapoints.len()));
    }
}
