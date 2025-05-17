pub fn fmt_with_pre_pad_till_9<T: ToString>(duration: &T) -> String {
    let dur_str = duration.to_string();
    if dur_str.len() > 9 {
        return dur_str;
    }
    let mut s = String::new();
    for _ in 0..(9_usize.saturating_sub(dur_str.len())) {
        s.push(' ');
    }
    s.push_str(&dur_str);
    s
}

pub fn empty_if_default<T: Default + ToString + PartialEq>(t: T) -> String {
    if t == Default::default() {
        "".to_string()
    } else {
        t.to_string()
    }
}
