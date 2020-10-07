use crate::ArchetypeAccess;
use bevy_hecs::{
    Archetype, Component, ComponentError, Entity, Fetch, Query as HecsQuery, Ref, RefMut, With,
    Without, World,
};
use bevy_tasks::ParallelIterator;
use std::marker::PhantomData;

/// Provides scoped access to a World according to a given [HecsQuery]
#[derive(Debug)]
pub struct Query<'a, Q: HecsQuery> {
    pub(crate) world: &'a World,
    pub(crate) archetype_access: &'a ArchetypeAccess,
    state: &'static (),
    _marker: PhantomData<Q>,
}

/// An error that occurs when using a [Query]
#[derive(Debug)]
pub enum QueryError {
    CannotReadArchetype,
    CannotWriteArchetype,
    ComponentError(ComponentError),
    NoSuchEntity,
}

impl<'w, Q: HecsQuery> Query<'w, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = ()>,
{
    #[inline]
    pub fn new(world: &'w World, archetype_access: &'w ArchetypeAccess) -> Self {
        Self {
            world,
            archetype_access,
            state: &(),
            _marker: PhantomData::default(),
        }
    }

    #[inline]
    pub fn iter(&mut self) -> QueryBorrowChecked<'w, 'static, (), Q::Fetch> {
        QueryBorrowChecked::new(&self.world.archetypes, self.archetype_access, &self.state)
    }

    // TODO: find a way to make `iter`, `get`, `get_mut`, and `entity` safe without using tracking pointers with global locks

    /// Gets a reference to the entity's component of the given type. This will fail if the entity does not have
    /// the given component type or if the given component type does not match this query.
    pub fn get<T: Component>(&self, entity: Entity) -> Result<Ref<T>, QueryError> {
        if let Some(location) = self.world.get_entity_location(entity) {
            if self
                .archetype_access
                .immutable
                .contains(location.archetype as usize)
                || self
                    .archetype_access
                    .mutable
                    .contains(location.archetype as usize)
            {
                // SAFE: we have already checked that the entity/component matches our archetype access. and systems are scheduled to run with safe archetype access
                unsafe {
                    self.world
                        .get_ref_at_location_unchecked(location)
                        .map_err(QueryError::ComponentError)
                }
            } else {
                Err(QueryError::CannotReadArchetype)
            }
        } else {
            Err(QueryError::ComponentError(ComponentError::NoSuchEntity))
        }
    }

    pub fn entity(&mut self, entity: Entity) -> Result<QueryOneChecked<'_, Q>, QueryError> {
        if let Some(location) = self.world.get_entity_location(entity) {
            if self
                .archetype_access
                .immutable
                .contains(location.archetype as usize)
                || self
                    .archetype_access
                    .mutable
                    .contains(location.archetype as usize)
            {
                // SAFE: we have already checked that the entity matches our archetype. and systems are scheduled to run with safe archetype access
                Ok(unsafe {
                    QueryOneChecked::new(
                        &self.world.archetypes[location.archetype as usize],
                        location.index,
                    )
                })
            } else {
                Err(QueryError::CannotReadArchetype)
            }
        } else {
            Err(QueryError::NoSuchEntity)
        }
    }

    /// Gets a mutable reference to the entity's component of the given type. This will fail if the entity does not have
    /// the given component type or if the given component type does not match this query.
    pub fn get_mut<T: Component>(&self, entity: Entity) -> Result<RefMut<'_, T>, QueryError> {
        let location = match self.world.get_entity_location(entity) {
            None => return Err(QueryError::ComponentError(ComponentError::NoSuchEntity)),
            Some(location) => location,
        };

        if self
            .archetype_access
            .mutable
            .contains(location.archetype as usize)
        {
            // SAFE: RefMut does exclusivity checks and we have already validated the entity
            unsafe {
                self.world
                    .get_ref_mut_at_location_unchecked(location)
                    .map_err(QueryError::ComponentError)
            }
        } else {
            Err(QueryError::CannotWriteArchetype)
        }
    }

    pub fn removed<C: Component>(&self) -> &[Entity] {
        self.world.removed::<C>()
    }

    /// Sets the entity's component to the given value. This will fail if the entity does not already have
    /// the given component type or if the given component type does not match this query.
    pub fn set<T: Component>(&mut self, entity: Entity, component: T) -> Result<(), QueryError> {
        let mut current = self.get_mut::<T>(entity)?;
        *current = component;
        Ok(())
    }
}

/// A borrow of a `World` sufficient to execute the query `Q`
///
/// Note that borrows are not released until this object is dropped.
pub struct QueryBorrowChecked<'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> {
    archetypes: &'w [Archetype],
    archetype_access: &'w ArchetypeAccess,
    state: &'s S,
    borrowed: bool,
    _marker: PhantomData<F>,
}

impl<'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>>
    QueryBorrowChecked<'w, 's, S, F>
{
    pub(crate) fn new(
        archetypes: &'w [Archetype],
        archetype_access: &'w ArchetypeAccess,
        state: &'s S,
    ) -> Self {
        Self {
            archetypes,
            borrowed: false,
            archetype_access,
            state,
            _marker: PhantomData,
        }
    }

    /// Execute the query
    ///
    /// Must be called only once per query.
    #[inline]
    pub fn iter<'q>(&'q mut self) -> QueryIter<'q, 'w, 's, S, F> {
        self.borrow();
        QueryIter {
            borrow: self,
            archetype_index: 0,
            iter: None,
        }
    }

    /// Like `iter`, but returns child iterators of at most `batch_size`
    /// elements
    ///
    /// Useful for distributing work over a threadpool using the
    /// ParallelIterator interface.
    ///
    /// Batch size needs to be chosen based on the task being done in
    /// parallel. The elements in each batch are computed serially, while
    /// the batches themselves are computed in parallel.
    ///
    /// A too small batch size can cause too much overhead, since scheduling
    /// each batch could take longer than running the batch. On the other
    /// hand, a too large batch size risks that one batch is still running
    /// long after the rest have finished.
    pub fn par_iter<'q>(&'q mut self, batch_size: usize) -> ParIter<'q, 'w, 's, S, F> {
        self.borrow();
        ParIter {
            borrow: self,
            archetype_index: 0,
            batch_size,
            batch: 0,
        }
    }

    fn borrow(&mut self) {
        if self.borrowed {
            panic!(
                "called QueryBorrowChecked::iter twice on the same borrow; construct a new query instead"
            );
        }

        for index in self.archetype_access.immutable.ones() {
            F::borrow(&self.archetypes[index], self.state);
        }

        for index in self.archetype_access.mutable.ones() {
            F::borrow(&self.archetypes[index], self.state);
        }

        self.borrowed = true;
    }
}

unsafe impl<'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Send
    for QueryBorrowChecked<'w, 's, S, F>
{
}
unsafe impl<'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Sync
    for QueryBorrowChecked<'w, 's, S, F>
{
}

impl<'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Drop
    for QueryBorrowChecked<'w, 's, S, F>
{
    #[inline]
    fn drop(&mut self) {
        if self.borrowed {
            for index in self.archetype_access.immutable.ones() {
                F::release(&self.archetypes[index], self.state);
            }

            for index in self.archetype_access.mutable.ones() {
                F::release(&self.archetypes[index], self.state);
            }
        }
    }
}

impl<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> IntoIterator
    for &'q mut QueryBorrowChecked<'w, 's, S, F>
{
    type IntoIter = QueryIter<'q, 'w, 's, S, F>;
    // FIXME: do I specify the concrete 'w? I tried to kind of avoid that a little with the `for<'a>
    // Fetch<'a>` trait bound.
    type Item = <F as Fetch<'q>>::Item;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> {
    borrow: &'q mut QueryBorrowChecked<'w, 's, S, F>,
    archetype_index: usize,
    iter: Option<ChunkIter<'s, S, F>>,
}

unsafe impl<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Send
    for QueryIter<'q, 'w, 's, S, F>
{
}
unsafe impl<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Sync
    for QueryIter<'q, 'w, 's, S, F>
{
}

impl<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Iterator
    for QueryIter<'q, 'w, 's, S, F>
{
    type Item = <F as Fetch<'q>>::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.borrow.archetypes.get(self.archetype_index as usize)?;
                    self.archetype_index += 1;
                    unsafe {
                        self.iter = F::get(archetype, 0, self.borrow.state).map(|fetch| ChunkIter {
                            fetch,
                            len: archetype.len(),
                            state: self.borrow.state,
                        });
                    }
                }
                Some(ref mut iter) => match unsafe { iter.next() } {
                    None => {
                        self.iter = None;
                        continue;
                    }
                    Some(components) => {
                        return Some(components);
                    }
                },
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.len();
        (n, Some(n))
    }
}

impl<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> ExactSizeIterator
    for QueryIter<'q, 'w, 's, S, F>
{
    fn len(&self) -> usize {
        self.borrow
            .archetypes
            .iter()
            .filter(|&x| F::access(x, self.borrow.state).is_some())
            .map(|x| x.len())
            .sum()
    }
}

struct ChunkIter<'s, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> {
    fetch: F,
    len: usize,
    state: &'s S,
}

impl<'s, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> ChunkIter<'s, S, F> {
    #[inline]
    unsafe fn next<'a>(&mut self) -> Option<<F as Fetch<'a>>::Item> {
        loop {
            if self.len == 0 {
                return None;
            }

            self.len -= 1;
            if self.fetch.should_skip(self.state) {
                // we still need to progress the iterator
                let _ = self.fetch.next(self.state);
                continue;
            }

            break Some(self.fetch.next(self.state));
        }
    }
}

/// Batched version of `QueryIter`
pub struct ParIter<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> {
    borrow: &'q mut QueryBorrowChecked<'w, 's, S, F>,
    archetype_index: usize,
    batch_size: usize,
    batch: usize,
}

impl<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>>
    ParallelIterator<Batch<'q, 's, S, F>> for ParIter<'q, 'w, 's, S, F>
{
    type Item = <F as Fetch<'q>>::Item;

    fn next_batch(&mut self) -> Option<Batch<'q, 's, S, F>> {
        loop {
            let archetype = self.borrow.archetypes.get(self.archetype_index)?;
            let offset = self.batch_size * self.batch;
            if offset >= archetype.len() {
                self.archetype_index += 1;
                self.batch = 0;
                continue;
            }
            if let Some(fetch) = unsafe { F::get(archetype, offset as usize, self.borrow.state) } {
                self.batch += 1;
                return Some(Batch {
                    _marker: PhantomData,
                    state: ChunkIter {
                        fetch,
                        len: self.batch_size.min(archetype.len() - offset),
                        state: self.borrow.state,
                    },
                });
            } else {
                self.archetype_index += 1;
                debug_assert_eq!(
                    self.batch, 0,
                    "query fetch should always reject at the first batch or not at all"
                );
                continue;
            }
        }
    }
}

/// A sequence of entities yielded by `ParIter`
pub struct Batch<'q, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> {
    _marker: PhantomData<&'q ()>,
    state: ChunkIter<'s, S, F>,
}

impl<'q, 'w, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Iterator
    for Batch<'q, 's, S, F>
{
    type Item = <F as Fetch<'q>>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let components = unsafe { self.state.next()? };
        Some(components)
    }
}

unsafe impl<'q, 's, S: Default + Sync + Send, F: for<'a> Fetch<'a, State = S>> Send
    for Batch<'q, 's, S, F>
{
}

/// A borrow of a `World` sufficient to execute the query `Q` on a single entity
pub struct QueryOneChecked<'a, Q: HecsQuery> {
    archetype: &'a Archetype,
    index: usize,
    borrowed: bool,
    _marker: PhantomData<Q>,
}

impl<'a, Q: HecsQuery> QueryOneChecked<'a, Q> {
    /// Construct a query accessing the entity in `archetype` at `index`
    ///
    /// # Safety
    ///
    /// `index` must be in-bounds for `archetype`
    pub(crate) unsafe fn new(archetype: &'a Archetype, index: usize) -> Self {
        Self {
            archetype,
            index,
            borrowed: false,
            _marker: PhantomData,
        }
    }

    /// Get the query result, or `None` if the entity does not satisfy the query
    ///
    /// Must be called at most once.
    ///
    /// Panics if called more than once or if it would construct a borrow that clashes with another
    /// pre-existing borrow.
    pub fn get(&mut self) -> Option<<Q::Fetch as Fetch<'_>>::Item> {
        unsafe {
            let mut fetch =
                Q::Fetch::get(self.archetype, self.index as usize, &Default::default())?;
            self.borrowed = true;
            Q::Fetch::borrow(self.archetype, &Default::default());
            Some(fetch.next(&Default::default()))
        }
    }

    /// Transform the query into one that requires a certain component without borrowing it
    ///
    /// See `QueryBorrow::with` for details.
    pub fn with<T: Component>(self) -> QueryOneChecked<'a, With<T, Q>> {
        self.transform()
    }

    /// Transform the query into one that skips entities having a certain component
    ///
    /// See `QueryBorrow::without` for details.
    pub fn without<T: Component>(self) -> QueryOneChecked<'a, Without<T, Q>> {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: HecsQuery>(self) -> QueryOneChecked<'a, R> {
        QueryOneChecked {
            archetype: self.archetype,
            index: self.index,
            borrowed: self.borrowed,
            _marker: PhantomData,
        }
    }
}

impl<Q: HecsQuery> Drop for QueryOneChecked<'_, Q> {
    fn drop(&mut self) {
        if self.borrowed {
            Q::Fetch::release(self.archetype, &Default::default());
        }
    }
}

unsafe impl<Q: HecsQuery> Send for QueryOneChecked<'_, Q> {}
unsafe impl<Q: HecsQuery> Sync for QueryOneChecked<'_, Q> {}
