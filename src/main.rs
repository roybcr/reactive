#![allow(dead_code)]

fn main() {}

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fmt::Debug,
    hash::Hash,
    rc::Rc,
};

type ComputeFn<T> = Box<dyn Fn(&[T]) -> T>;
type CallbackFn<'a, T> = Box<dyn FnMut(T) + 'a>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InputCellId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ComputeCellId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackId(pub usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CellId {
    Input(InputCellId),
    Compute(ComputeCellId),
}

impl CellId {
    pub fn get_id(&self) -> usize {
        match self {
            CellId::Input(InputCellId(id)) => *id,
            CellId::Compute(ComputeCellId(id)) => *id,
        }
    }

    fn is_compute(&self) -> bool {
        match self {
            CellId::Input(_) => false,
            CellId::Compute(_) => true,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum RemoveCallbackError {
    NonexistentCell,
    NonexistentCallback,
}

pub enum Callback<'a, T>
where T: Copy + PartialEq + Debug
{
    Removed,
    Exists(CallbackFn<'a, T>),
}

impl<'a, T> Debug for Callback<'a, T>
where T: Copy + PartialEq + Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Callback::Removed => f.write_fmt(format_args!("Removed")),
            Callback::Exists(_) => f.write_fmt(format_args!("Exists")),
        }
    }
}

pub enum Compute<T>
where T: Copy + PartialEq + Debug
{
    Identity(T),
    Computed(ComputeFn<T>),
}

impl<T> Compute<T>
where T: Copy + PartialEq + Debug
{
    fn new_identity(value: T) -> Self {
        Compute::Identity(value)
    }
    fn new_computed(func: ComputeFn<T>) -> Self {
        Compute::Computed(func)
    }
    fn get_computed(&self, deps: &[T]) -> T {
        match self {
            Compute::Identity(value) => *value,
            Compute::Computed(func) => func(deps),
        }
    }
}

pub struct RxCell<'a, T>
where T: Copy + PartialEq + Debug + 'a
{
    pub id:        CellId,
    pub val:       Cell<T>,
    pub deps:      HashSet<CellId>,
    pub callbacks: RefCell<Vec<Callback<'a, T>>>,
}

impl<'a, T> RxCell<'a, T>
where T: Copy + PartialEq + Debug + 'a
{
    pub fn new(id: CellId, val: T, deps: &[CellId]) -> Self {
        let mut set = HashSet::new();

        for &dep in deps {
            set.insert(dep);
        }

        RxCell {
            id,
            deps: set,
            val: Cell::new(val),
            callbacks: RefCell::new(vec![]),
        }
    }

    pub fn is_compute(&self) -> bool {
        self.id.is_compute()
    }

    pub fn get_value(&self) -> T {
        self.val.get()
    }
}

impl<'a, T> Debug for RxCell<'a, T>
where T: Copy + PartialEq + Debug + 'a
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RxCell")
            .field("ID", &self.id)
            .field("Value", &self.val)
            .field("Dependencies", &self.deps)
            .finish()
    }
}

pub struct Reactor<'a, T>
where T: Copy + PartialEq + Debug
{
    pub cells: Vec<RxCell<'a, T>>,
    pub funcs: Vec<Rc<Compute<T>>>,
    pub graph: HashMap<usize, Vec<CellId>>,
    pub cache: RefCell<HashMap<CellId, T>>,
}

impl<T> Debug for Reactor<'_, T>
where T: Copy + PartialEq + Debug
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reactor")
            .field("cells", &self.cells)
            .field("graph", &self.graph)
            .finish()
    }
}

// You are guaranteed that Reactor will only be tested against types that are
// Copy + PartialEq.
impl<'a, T: Copy + PartialEq + Debug> Reactor<'a, T> {
    pub fn new() -> Self {
        Reactor {
            cells: vec![],
            funcs: vec![],
            cache: RefCell::new(HashMap::new()),
            graph: HashMap::<usize, Vec<CellId>>::new(),
        }
    }

    pub fn create_input(&mut self, initial: T) -> InputCellId {
        let idx = self.cells.len();
        let iid = InputCellId(idx);
        let cid = CellId::Input(iid.clone());

        self.funcs.push(Rc::new(Compute::new_identity(initial)));
        self.cells.push(RxCell::new(cid, initial, &[]));
        (*self.cache.borrow_mut()).insert(cid, initial);

        iid
    }

    pub fn create_compute<F: Fn(&[T]) -> T + 'static>(
        &mut self,
        dependencies: &[CellId],
        compute_func: F,
    ) -> Result<ComputeCellId, CellId> {
        let mut vals = vec![];
        let len = self.cells.len();
        let iid = ComputeCellId(len);
        let cid = CellId::Compute(iid);

        for cell_id in dependencies[..].iter() {
            if let Some(cached_entry) = self.cache.borrow().get(cell_id) {
                vals.push(*cached_entry);
            } else {
                match self.cells.get(cell_id.get_id()) {
                    Some(cell) => vals.push(cell.get_value()),
                    None => {
                        return Err(*cell_id);
                    }
                };
            }

            self.graph
                .entry(cell_id.get_id())
                .and_modify(|cells| (*cells).push(cid))
                .or_insert(vec![cid]);
        }

        let computed_value = compute_func(&vals);
        let boxed_cb = Box::new(compute_func);

        self.funcs.push(Rc::new(Compute::new_computed(boxed_cb)));
        self.cells
            .push(RxCell::new(cid, computed_value, dependencies));

        Ok(iid)
    }

    pub fn value(&self, id: CellId) -> Option<T> {
        // attempting to retrieve a cell's value from the cache, or calculating
        // its value and updating the cache with the result.
        (*self.cache.borrow()).get(&id).map_or_else(
            || self.cells.get(id.get_id()).map(|c| c.get_value()),
            |&entry| {
                self.set(id, entry);
                Some(entry)
            },
        )
    }

    pub fn set_value(&self, id: InputCellId, new_value: T) -> bool {
        let cell_id = CellId::Input(id);
        let InputCellId(idx) = id;
        match self.cells.get(idx) {
            None => false,
            Some(cell) => {
                cell.val.set(new_value);
                (*self.cache.borrow_mut()).insert(cell_id, new_value);
                for dep in self.get_dependants(cell_id) {
                    if let Some(dep_cell) = self.cells.get(dep.get_id()) {
                        if dep_cell.deps.contains(&cell_id) {
                            // INFO:
                            // In order to re-compute `dep_cell`, we need to
                            // retrieve the values of all its dependencies, then
                            // invoke its compute function with those values.
                            //
                            // NOTE:
                            // For any dependency of `dep_cell`, if a dependency
                            // is a compute cell as well, we want to check
                            // whether it's also depends on the current input
                            // cell, and in case it does - to recompute it as
                            // well, before `dep_cell`, since its new computed
                            // value is required to compute the new `dep_cell`
                            // value. In case it doesn't, its value can simply
                            // be retrieved from the cache.
                            self.deep_compute(&dep_cell, cell_id);
                        }
                    }
                }

                true
            }
        }
    }

    fn get_dependants(&self, id: CellId) -> Vec<CellId> {
        self.graph.get(&id.get_id()).map_or(vec![], |d| d.clone())
    }

    fn deep_compute(&self, cell: &RxCell<'a, T>, changed_cell: CellId) -> T {
        if !cell.is_compute() {
            return cell.get_value();
        }

        let mut seen = HashSet::new();
        let mut changed = HashSet::new();

        fn deep_compute_inner<'a, T>(
            cell: &RxCell<'a, T>,
            reactor: &Reactor<'a, T>,
            changed_cell: CellId,
            seen: &mut HashSet<CellId>,
            changed: &mut HashSet<CellId>,
        ) -> T
        where
            T: Copy + PartialEq + Debug,
        {
            let mut vals = vec![];
            for dep_id in cell.deps.iter() {
                if seen.contains(&dep_id) {
                    vals.push(reactor.value(*dep_id).unwrap());
                    continue;
                } else {
                    seen.insert(*dep_id);
                }

                if let Some(dep_cell) = reactor.cells.get(dep_id.get_id()) {
                    if !dep_cell.is_compute()
                        || !(*dep_cell).deps.contains(&dep_id)
                    {
                        vals.push(reactor.value(*dep_id).unwrap());
                        continue;
                    }

                    let dep_computed_value = deep_compute_inner(
                        dep_cell,
                        reactor,
                        changed_cell,
                        seen,
                        changed,
                    );

                    if dep_cell.get_value().ne(&dep_computed_value) {
                        changed.insert(*dep_id);
                        dep_cell.val.set(dep_computed_value);
                        reactor.set(*dep_id, dep_computed_value);
                    }

                    vals.push(dep_computed_value);
                }
            }

            let rccb = reactor.funcs.get(cell.id.get_id()).unwrap();
            let new_value = Rc::clone(rccb).get_computed(&vals);

            if cell.get_value().ne(&new_value) {
                changed.insert(cell.id);
                cell.val.set(new_value);
                reactor.set(cell.id, new_value);
            }

            for dependent in reactor.get_dependants(cell.id) {
                let cell_to_compute =
                    reactor.cells.get(dependent.get_id()).unwrap();

                deep_compute_inner(
                    &cell_to_compute,
                    reactor,
                    cell.id,
                    seen,
                    changed,
                );
            }

            for changed_cell in changed.iter() {
                reactor.invoke_callbacks(*changed_cell);
            }

            new_value
        }

        deep_compute_inner(cell, self, changed_cell, &mut seen, &mut changed)
    }

    fn set(&self, k: CellId, v: T) {
        let _ = self
            .cache
            .try_borrow_mut()
            .as_deref_mut()
            .map(|cache| cache.insert(k, v));
    }

    pub fn invoke_callbacks(&self, id: CellId) {
        if let Some(cell) = self.cells.get(id.get_id()) {
            let cell_value = self.value(id).unwrap();
            for (i, cb) in cell
                .callbacks
                .borrow_mut()
                .as_mut_slice()
                .into_iter()
                .enumerate()
            {
                println!("{i}");
                match cb {
                    Callback::Removed => {}
                    Callback::Exists(cb) => {
                        cb(cell_value);
                    }
                }
            }
        }
    }

    // Adds a callback to the specified compute cell.
    // Returns the ID of the just-added callback, or None if the cell doesn't
    // exist.
    // Callbacks on input cells will not be tested.
    // The semantics of callbacks (as will be tested):
    // For a single set_value call,
    // each compute cell's callbacks should each be called:
    //
    //    * Zero times if the compute cell's value did not change as a result of
    //      the set_value call.
    //    * Exactly once if the compute cell's value changed as a result of the
    //      set_value call. The value passed to the callback should be the final
    //      value of the compute cell after the set_value call.
    pub fn add_callback<F: FnMut(T) + 'a>(
        &mut self,
        id: ComputeCellId,
        callback: F,
    ) -> Option<CallbackId> {
        let ComputeCellId(idx) = id;
        if let Some(cell) = self.cells.get(idx) {
            let callback_id = CallbackId(cell.callbacks.borrow().len());
            (*cell.callbacks.borrow_mut())
                .push(Callback::Exists(Box::new(callback)));
            return Some(callback_id);
        }

        None
    }

    // Removes the specified callback, using an ID returned from
    // add_callback.
    // Returns an Err if either the cell or callback does not exist.
    // A removed callback should no longer be called.
    pub fn remove_callback(
        &mut self,
        cell: ComputeCellId,
        callback: CallbackId,
    ) -> Result<(), RemoveCallbackError> {
        let ComputeCellId(cell_idx) = cell;
        let CallbackId(cb_idx) = callback;

        match self.cells.get(cell_idx) {
            None => Err(RemoveCallbackError::NonexistentCell),
            Some(cell) => {
                if let Some(cb) = (*cell.callbacks.borrow_mut()).get_mut(cb_idx)
                {
                    *cb = Callback::Removed;
                    println!("{:?}", cb);
                    return Ok(());
                }

                Err(RemoveCallbackError::NonexistentCallback)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_computed_cells_update_when_dep_changes() {
        let mut reactor = Reactor::<i32>::new();
        let i1 = reactor.create_input(10);
        let i2 = reactor.create_input(20);
        let c1 = reactor.create_compute(
            &[CellId::Input(i1), CellId::Input(i2)],
            |cells: &[i32]| cells.iter().sum(),
        );

        match c1 {
            Ok(id) => {
                let value = reactor.value(CellId::Compute(id));
                assert_eq!(Some(30), value);
            }

            Err(_) => {
                println!("Error matching c1");
                return;
            }
        }

        reactor.set_value(i1, 50);
        assert_eq!(reactor.value(CellId::Compute(c1.unwrap())), Some(70));
    }

    #[test]
    fn input_cells_have_a_value() {
        let mut reactor = Reactor::new();
        let input = reactor.create_input(10);

        assert_eq!(reactor.value(CellId::Input(input)), Some(10));
    }

    #[test]
    fn an_input_cells_value_can_be_set() {
        let mut reactor = Reactor::new();
        let input = reactor.create_input(4);

        assert!(reactor.set_value(input, 20));
        assert_eq!(reactor.value(CellId::Input(input)), Some(20));
    }

    #[test]
    fn error_setting_a_nonexistent_input_cell() {
        let mut dummy_reactor = Reactor::new();
        let input = dummy_reactor.create_input(1);

        assert!(!Reactor::new().set_value(input, 0));
    }

    #[test]
    fn compute_cells_calculate_initial_value() {
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();

        assert_eq!(reactor.value(CellId::Compute(output)), Some(2));
    }

    #[test]
    fn compute_cells_take_inputs_in_the_right_order() {
        let mut reactor = Reactor::new();
        let one = reactor.create_input(1);
        let two = reactor.create_input(2);
        let output = reactor
            .create_compute(&[CellId::Input(one), CellId::Input(two)], |v| {
                v[0] + v[1] * 10
            })
            .unwrap();

        assert_eq!(reactor.value(CellId::Compute(output)), Some(21));
    }

    #[test]
    fn error_creating_compute_cell_if_input_doesnt_exist() {
        let mut dummy_reactor = Reactor::new();
        let input = dummy_reactor.create_input(1);

        assert_eq!(
            Reactor::new().create_compute(&[CellId::Input(input)], |_| 0),
            Err(CellId::Input(input))
        );
    }

    #[test]
    fn do_not_break_cell_if_creating_compute_cell_with_valid_and_invalid_input()
    {
        let mut dummy_reactor = Reactor::new();
        let _ = dummy_reactor.create_input(1);
        let dummy_cell = dummy_reactor.create_input(2);
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);

        assert_eq!(
            reactor.create_compute(
                &[CellId::Input(input), CellId::Input(dummy_cell)],
                |_| 0
            ),
            Err(CellId::Input(dummy_cell))
        );

        assert!(reactor.set_value(input, 5));
        assert_eq!(reactor.value(CellId::Input(input)), Some(5));
    }

    #[test]
    fn compute_cells_update_value_when_dependencies_are_changed() {
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();

        assert_eq!(reactor.value(CellId::Compute(output)), Some(2));
        assert!(reactor.set_value(input, 3));
        assert_eq!(reactor.value(CellId::Compute(output)), Some(4));
    }

    #[test]
    fn compute_cells_can_depend_on_other_compute_cells() {
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let times_two = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] * 2)
            .unwrap();
        let times_thirty = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] * 30)
            .unwrap();
        let output = reactor
            .create_compute(
                &[CellId::Compute(times_two), CellId::Compute(times_thirty)],
                |v| v[0] + v[1],
            )
            .unwrap();

        assert_eq!(reactor.value(CellId::Compute(output)), Some(32));
        assert!(reactor.set_value(input, 3));
        assert_eq!(reactor.value(CellId::Compute(output)), Some(96));
    }

    /// A CallbackRecorder helps tests whether callbacks get called
    /// correctly.
    /// You'll see it used in tests that deal with callbacks.
    /// The names should be descriptive enough so that the tests make sense,
    /// so it's not necessary to fully understand the implementation,
    /// though you are welcome to.

    struct CallbackRecorder {
        // Note that this `Cell` is https://doc.rust-lang.org/std/cell/
        // a mechanism to allow internal mutability,
        // distinct from the cells (input cells, compute cells) in the
        // reactor
        value: std::cell::Cell<Option<i32>>,
    }

    impl CallbackRecorder {
        fn new() -> Self {
            CallbackRecorder {
                value: std::cell::Cell::new(None),
            }
        }

        fn expect_to_have_been_called_with(&self, v: i32) {
            assert_ne!(
                self.value.get(),
                None,
                "Callback was not called, but should have been"
            );
            assert_eq!(
                self.value.replace(None),
                Some(v),
                "Callback was called with incorrect value"
            );
        }

        fn expect_not_to_have_been_called(&self) {
            assert_eq!(
                self.value.get(),
                None,
                "Callback was called, but should not have been"
            );
        }

        fn callback_called(&self, v: i32) {
            assert_eq!(
                self.value.replace(Some(v)),
                None,
                "Callback was called too many times; can't be called with {}",
                v
            );
        }
    }

    #[test]
    fn compute_cells_fire_callbacks() {
        let cb = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();

        assert!(reactor
            .add_callback(output, |v| cb.callback_called(v))
            .is_some());
        assert!(reactor.set_value(input, 3));
        cb.expect_to_have_been_called_with(4);
    }

    #[test]
    fn error_adding_callback_to_nonexistent_cell() {
        let mut dummy_reactor = Reactor::new();
        let input = dummy_reactor.create_input(1);
        let output = dummy_reactor
            .create_compute(&[CellId::Input(input)], |_| 0)
            .unwrap();

        assert_eq!(
            Reactor::new().add_callback(output, |_: u32| println!("hi")),
            None
        );
    }

    #[test]
    fn error_removing_callback_from_nonexisting_cell() {
        let mut dummy_reactor = Reactor::new();
        let dummy_input = dummy_reactor.create_input(1);
        let _ = dummy_reactor
            .create_compute(&[CellId::Input(dummy_input)], |_| 0)
            .unwrap();
        let dummy_output = dummy_reactor
            .create_compute(&[CellId::Input(dummy_input)], |_| 0)
            .unwrap();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |_| 0)
            .unwrap();
        let callback = reactor.add_callback(output, |_| ()).unwrap();

        assert_eq!(
            reactor.remove_callback(dummy_output, callback),
            Err(RemoveCallbackError::NonexistentCell)
        );
    }

    #[test]
    fn callbacks_only_fire_on_change() {
        let cb = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |v| {
                if v[0] < 3 {
                    111
                } else {
                    222
                }
            })
            .unwrap();

        assert!(reactor
            .add_callback(output, |v| cb.callback_called(v))
            .is_some());
        assert!(reactor.set_value(input, 2));
        cb.expect_not_to_have_been_called();
        assert!(reactor.set_value(input, 4));
        cb.expect_to_have_been_called_with(222);
    }

    #[test]
    fn callbacks_can_be_called_multiple_times() {
        let cb = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();

        assert!(reactor
            .add_callback(output, |v| cb.callback_called(v))
            .is_some());
        assert!(reactor.set_value(input, 2));
        cb.expect_to_have_been_called_with(3);
        assert!(reactor.set_value(input, 3));
        cb.expect_to_have_been_called_with(4);
    }

    #[test]
    fn callbacks_can_be_called_from_multiple_cells() {
        let cb1 = CallbackRecorder::new();
        let cb2 = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let plus_one = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();
        let minus_one = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] - 1)
            .unwrap();

        assert!(reactor
            .add_callback(plus_one, |v| cb1.callback_called(v))
            .is_some());
        assert!(reactor
            .add_callback(minus_one, |v| cb2.callback_called(v))
            .is_some());
        assert!(reactor.set_value(input, 10));

        cb1.expect_to_have_been_called_with(11);
        cb2.expect_to_have_been_called_with(9);
    }

    #[test]
    fn callbacks_can_be_added_and_removed() {
        let cb1 = CallbackRecorder::new();
        let cb2 = CallbackRecorder::new();
        let cb3 = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(11);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();
        let callback = reactor
            .add_callback(output, |v| cb1.callback_called(v))
            .unwrap();

        assert!(reactor
            .add_callback(output, |v| cb2.callback_called(v))
            .is_some());
        assert!(reactor.set_value(input, 31));

        cb1.expect_to_have_been_called_with(32);
        cb2.expect_to_have_been_called_with(32);

        assert!(reactor.remove_callback(output, callback).is_ok());
        assert!(reactor
            .add_callback(output, |v| cb3.callback_called(v))
            .is_some());
        assert!(reactor.set_value(input, 41));

        cb1.expect_not_to_have_been_called();
        cb2.expect_to_have_been_called_with(42);
        cb3.expect_to_have_been_called_with(42);
    }

    #[test]
    fn removing_a_callback_multiple_times_doesnt_interfere_with_other_callbacks(
    ) {
        let cb1 = CallbackRecorder::new();
        let cb2 = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let output = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();
        let callback = reactor
            .add_callback(output, |v| cb1.callback_called(v))
            .unwrap();

        assert!(reactor
            .add_callback(output, |v| cb2.callback_called(v))
            .is_some());

        // We want the first remove to be Ok, but the others should be
        // errors.

        assert!(reactor.remove_callback(output, callback).is_ok());

        for _ in 1..5 {
            assert_eq!(
                reactor.remove_callback(output, callback),
                Err(RemoveCallbackError::NonexistentCallback)
            );
        }

        assert!(reactor.set_value(input, 2));
        cb1.expect_not_to_have_been_called();
        cb2.expect_to_have_been_called_with(3);
    }

    #[test]
    fn callbacks_should_only_be_called_once_even_if_multiple_dependencies_change(
    ) {
        let cb = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let plus_one = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();
        let minus_one1 = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] - 1)
            .unwrap();
        let minus_one2 = reactor
            .create_compute(&[CellId::Compute(minus_one1)], |v| v[0] - 1)
            .unwrap();
        let output = reactor
            .create_compute(
                &[CellId::Compute(plus_one), CellId::Compute(minus_one2)],
                |v| v[0] * v[1],
            )
            .unwrap();

        assert!(reactor
            .add_callback(output, |v| cb.callback_called(v))
            .is_some());
        assert!(reactor.set_value(input, 4));
        cb.expect_to_have_been_called_with(10);
    }

    #[test]
    fn callbacks_should_not_be_called_if_dependencies_change_but_output_value_doesnt_change(
    ) {
        let cb = CallbackRecorder::new();
        let mut reactor = Reactor::new();
        let input = reactor.create_input(1);
        let plus_one = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] + 1)
            .unwrap();
        let minus_one = reactor
            .create_compute(&[CellId::Input(input)], |v| v[0] - 1)
            .unwrap();
        let always_two = reactor
            .create_compute(
                &[CellId::Compute(plus_one), CellId::Compute(minus_one)],
                |v| v[0] - v[1],
            )
            .unwrap();

        assert!(reactor
            .add_callback(always_two, |v| cb.callback_called(v))
            .is_some());
        for i in 2..5 {
            assert!(reactor.set_value(input, i));
            cb.expect_not_to_have_been_called();
        }
    }

    #[test]
    fn test_adder_with_boolean_values() {
        // This is a digital logic circuit called an adder:
        // https://en.wikipedia.org/wiki/Adder_(electronics)

        let mut reactor = Reactor::new();
        let a = reactor.create_input(false);
        let b = reactor.create_input(false);
        let carry_in = reactor.create_input(false);
        let a_xor_b = reactor
            .create_compute(&[CellId::Input(a), CellId::Input(b)], |v| {
                v[0] ^ v[1]
            })
            .unwrap();
        let sum = reactor
            .create_compute(
                &[CellId::Compute(a_xor_b), CellId::Input(carry_in)],
                |v| v[0] ^ v[1],
            )
            .unwrap();
        let a_xor_b_and_cin = reactor
            .create_compute(
                &[CellId::Compute(a_xor_b), CellId::Input(carry_in)],
                |v| v[0] && v[1],
            )
            .unwrap();
        let a_and_b = reactor
            .create_compute(&[CellId::Input(a), CellId::Input(b)], |v| {
                v[0] && v[1]
            })
            .unwrap();
        let carry_out = reactor
            .create_compute(
                &[CellId::Compute(a_xor_b_and_cin), CellId::Compute(a_and_b)],
                |v| v[0] || v[1],
            )
            .unwrap();

        let tests = &[
            (false, false, false, false, false),
            (false, false, true, false, true),
            (false, true, false, false, true),
            (false, true, true, true, false),
            (true, false, false, false, true),
            (true, false, true, true, false),
            (true, true, false, true, false),
            (true, true, true, true, true),
        ];

        for &(aval, bval, cinval, expected_cout, expected_sum) in tests {
            assert!(reactor.set_value(a, aval));
            assert!(reactor.set_value(b, bval));
            assert!(reactor.set_value(carry_in, cinval));
            assert_eq!(reactor.value(CellId::Compute(sum)), Some(expected_sum));
            assert_eq!(
                reactor.value(CellId::Compute(carry_out)),
                Some(expected_cout)
            );
        }
    }
}
