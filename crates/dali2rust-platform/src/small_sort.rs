pub fn insertion_sort_by<T>(items: &mut [T], is_after: impl Fn(&T, &T) -> bool) {
    for end in 1..items.len() {
        let mut at = end;
        while at > 0 && is_after(&items[at - 1], &items[at]) {
            items.swap(at - 1, at);
            at -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_come_out_in_order() {
        let mut items = [5, 1, 4, 2, 3, 0];
        insertion_sort_by(&mut items, |a, b| a > b);
        assert_eq!(items, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn equal_keys_keep_their_input_order() {
        let mut items = [(2, 'a'), (1, 'b'), (2, 'c'), (1, 'd')];
        insertion_sort_by(&mut items, |a, b| a.0 > b.0);
        assert_eq!(items, [(1, 'b'), (1, 'd'), (2, 'a'), (2, 'c')]);
    }

    #[test]
    fn an_empty_or_single_slice_is_left_alone() {
        let mut empty: [u8; 0] = [];
        insertion_sort_by(&mut empty, |a, b| a > b);
        let mut one = [7];
        insertion_sort_by(&mut one, |a, b| a > b);
        assert_eq!(one, [7]);
    }
}
