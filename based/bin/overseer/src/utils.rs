pub fn empty_if_default<T: Default + ToString + PartialEq>(t: T) -> String {
    if t == Default::default() { "".to_string() } else { t.to_string() }
}
