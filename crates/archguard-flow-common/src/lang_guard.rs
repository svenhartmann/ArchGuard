#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn blocks_and_brackets_open_at_an_offset() {
        let s = "a { b ( c { d";
        assert_eq!(open_blocks(s.as_bytes(), 0, s.len()), vec![2, 10]);
        assert_eq!(innermost_open(s.as_bytes(), 0, s.len()), Some(10));
        assert_eq!(innermost_open(b"{ ( ) }", 0, 7), None);
    }

    #[test]
    fn statement_end_stops_at_semicolon_or_line_break_at_depth_zero() {
        assert_eq!(statement_end(b"f(a;\nb); x", 0), Some(8));
        assert_eq!(statement_end(b"a = 1\nb;", 0), Some(5));
    }

    #[test]
    fn if_at_reads_braced_and_single_statements() {
        let s = "if (x) { y }";
        let (c, body) = if_at(s, 0).unwrap();
        assert_eq!((&s[c.0..c.1], &s[body.0..=body.1]), ("x", "{ y }"));
        let s = "if (x) a = 1\nb = 2;";
        let (_, body) = if_at(s, 0).unwrap();
        assert_eq!(&s[body.0..body.1], "a = 1");
        assert!(if_at("iff (x) a;", 0).is_none());
        assert!(if_at("xif (x) a;", 1).is_none());
    }

    #[test]
    fn exits_on_throw_return_or_a_throwing_helper() {
        let helpers = BTreeSet::from(["fail"]);
        assert!(exits("throw x", (0, 7), &helpers));
        assert!(exits("fail()", (0, 6), &helpers));
        assert!(!exits("x = 1", (0, 5), &helpers));
    }

    #[test]
    fn calls_in_keeps_unique_owners_and_reports_ambiguous_ones() {
        let owners = BTreeMap::from([
            ("ship", BTreeSet::from(["Order"])),
            ("pay", BTreeSet::from(["A", "B"])),
        ]);
        let s = "o.ship(); o.pay(); ship(); x.other();";
        let (calls, amb) = calls_in(s, (0, s.len()), &owners);
        assert_eq!(calls, vec!["Order.ship"]);
        assert_eq!(amb.len(), 1);
        assert_eq!(amb[0].0, 12);
        assert_eq!(
            amb[0].1,
            "call .pay( matches methods of several entities (A, B); not recorded."
        );
    }

    #[test]
    fn capitalize_uppercases_the_first_char_only() {
        assert_eq!(capitalize("orderId"), "OrderId");
        assert_eq!(capitalize(""), "");
    }

    fn new_x(x: &str) -> Option<String> {
        x.trim()
            .strip_prefix("new ")
            .map(|r| r.trim_end_matches("()").to_string())
    }

    #[test]
    fn created_reads_a_new_expression_or_a_variable_assigned_one() {
        assert_eq!(created("", (0, 0), "new E()", new_x), Some("E".into()));
        let s = "var e = new E(); raise(e)";
        assert_eq!(created(s, (0, 16), "e", new_x), Some("E".into()));
        assert_eq!(created(s, (0, 16), "", new_x), None);
        assert_eq!(created(s, (0, 16), "f(1)", new_x), None);
    }
}
