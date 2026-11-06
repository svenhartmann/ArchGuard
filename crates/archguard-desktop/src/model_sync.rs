//! Slint list models updated in place. A view that gets a new `ModelRc`
//! makes its repeater drop and rebuild every item; one that keeps its
//! model and sees only the rows that changed rebuilds nothing and
//! redraws only those rows.

use slint::{Model, ModelRc, VecModel};

/// `old` holding `rows`, if it is a `VecModel`: rows equal to the old
/// ones stay untouched, others are set, the rest added or removed. Any
/// other model (e.g. the default empty one) is replaced by a new one.
pub fn sync_rows<T: Clone + PartialEq + 'static>(
    old: &ModelRc<T>,
    rows: impl IntoIterator<Item = T>,
) -> ModelRc<T> {
    sync_rows_with(old, rows, |_, _| {})
}

/// [`sync_rows`] for rows that hold models themselves: `adopt(old, new)`
/// lets the new row take over the old row's inner models (synced with
/// [`sync_rows`]) before both are compared.
pub fn sync_rows_with<T: Clone + PartialEq + 'static>(
    old: &ModelRc<T>,
    rows: impl IntoIterator<Item = T>,
    adopt: impl Fn(&T, &mut T),
) -> ModelRc<T> {
    let Some(vec) = old.as_any().downcast_ref::<VecModel<T>>() else {
        return ModelRc::new(VecModel::from(rows.into_iter().collect::<Vec<_>>()));
    };
    let mut count = 0;
    for (i, mut row) in rows.into_iter().enumerate() {
        count = i + 1;
        match vec.row_data(i) {
            Some(prev) => {
                adopt(&prev, &mut row);
                if prev != row {
                    vec.set_row_data(i, row);
                }
            }
            None => vec.push(row),
        }
    }
    while vec.row_count() > count {
        vec.remove(vec.row_count() - 1);
    }
    old.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    /// A row equal by value whose identity shows whether the model kept
    /// it or was given a new one.
    #[derive(Clone)]
    struct Row(i32, Rc<()>);

    impl PartialEq for Row {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }

    fn rows(values: &[i32]) -> Vec<Row> {
        values.iter().map(|&v| Row(v, Rc::new(()))).collect()
    }

    fn values(m: &ModelRc<Row>) -> Vec<i32> {
        m.iter().map(|r| r.0).collect()
    }

    /// Unchanged rows stay the very rows they were; the model stays the
    /// same one, so its repeater keeps its items.
    #[test]
    fn only_changed_rows_are_set() {
        let model = ModelRc::new(VecModel::from(rows(&[1, 2, 3])));
        let before: Vec<Row> = model.iter().collect();
        let same = sync_rows(&model, rows(&[1, 5, 3, 4]));
        assert_eq!(same, model, "the same model");
        assert_eq!(values(&model), [1, 5, 3, 4]);
        let kept = |i: usize| Rc::ptr_eq(&model.row_data(i).expect("row").1, &before[i].1);
        assert!(kept(0) && kept(2), "equal rows untouched");
        assert!(!kept(1), "a changed row is set");
        sync_rows(&model, rows(&[1]));
        assert_eq!(values(&model), [1]);
        assert!(kept(0));
    }

    /// The default (empty) model is replaced by a `VecModel`, which the
    /// next sync keeps.
    #[test]
    fn a_default_model_is_replaced_once() {
        let first = sync_rows(&ModelRc::default(), rows(&[1, 2]));
        assert_eq!(values(&first), [1, 2]);
        assert_eq!(sync_rows(&first, rows(&[2])), first);
    }

    /// A row with an inner model keeps it, synced in place.
    #[test]
    fn inner_models_are_adopted() {
        let inner = |v: &[i32]| ModelRc::new(VecModel::from(v.to_vec()));
        let outer: ModelRc<ModelRc<i32>> = ModelRc::new(VecModel::from(vec![inner(&[1, 2])]));
        let kept = outer.row_data(0).expect("row");
        sync_rows_with(&outer, [inner(&[1, 3])], |old, new| {
            *new = sync_rows(old, new.iter());
        });
        let after = outer.row_data(0).expect("row");
        assert_eq!(after, kept, "the inner model stays");
        assert_eq!(after.iter().collect::<Vec<_>>(), [1, 3]);
    }
}
