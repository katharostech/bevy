use crate::ArchetypeAccess;
use bevy_hecs::{
    Archetype, Component, ComponentError, Entity, Fetch, Query as HecsQuery, Ref, RefMut, With,
    Without, World,
};
use bevy_tasks::ParallelIterator;
use std::marker::PhantomData;

pub type Query<'a, Q> = GenericQuery<'a, 'static, (), Q>;

/// Provides scoped access to a World according to a given [HecsQuery]
#[derive(Debug)]
pub struct GenericQuery<'w, 's, S, Q: HecsQuery> {
    pub(crate) world: &'w World,
    pub(crate) archetype_access: &'w ArchetypeAccess,
    state: &'s S,
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

impl<'w, Q: HecsQuery> GenericQuery<'w, 'static, (), Q> {
    #[inline]
    pub fn new(
        world: &'w World,
        archetype_access: &'w ArchetypeAccess,
    ) -> GenericQuery<'w, 'static, (), Q> {
        Self::new_stateless(world, archetype_access)
    }

    #[inline]
    fn new_stateless(world: &'w World, archetype_access: &'w ArchetypeAccess) -> Self {
        Self {
            world,
            archetype_access,
            state: &(),
            _marker: PhantomData::default(),
        }
    }
}

impl<'w, 's, S: Default, Q: HecsQuery> GenericQuery<'w, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    #[inline]
    pub fn new_stateful(
        world: &'w World,
        archetype_access: &'w ArchetypeAccess,
        state: &'s S,
    ) -> GenericQuery<'w, 's, S, Q> {
        GenericQuery {
            world,
            archetype_access,
            state,
            _marker: PhantomData::default(),
        }
    }

    #[inline]
    pub fn iter(&mut self) -> QueryBorrowChecked<'_, 's, S, Q> {
        QueryBorrowChecked::new(&self.world.archetypes, self.archetype_access, self.state)
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

    pub fn entity(&mut self, entity: Entity) -> Result<QueryOneChecked<'w, 's, S, Q>, QueryError> {
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
                        self.state,
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
pub struct QueryBorrowChecked<'w, 's, S: Default, Q: HecsQuery>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    archetypes: &'w [Archetype],
    archetype_access: &'w ArchetypeAccess,
    borrowed: bool,
    state: &'s S,
    _marker: PhantomData<Q>,
}

impl<'w, 's, S: Default, Q: HecsQuery> QueryBorrowChecked<'w, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
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
    pub fn iter<'q>(&'q mut self) -> QueryIter<'q, 'w, 's, S, Q> {
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
    pub fn par_iter<'q>(&'q mut self, batch_size: usize) -> ParIter<'q, 'w, 's, S, Q> {
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
            Q::Fetch::borrow(&self.archetypes[index], self.state);
        }

        for index in self.archetype_access.mutable.ones() {
            Q::Fetch::borrow(&self.archetypes[index], self.state);
        }

        self.borrowed = true;
    }
}

unsafe impl<'w, 's, S: Default, Q: HecsQuery> Send for QueryBorrowChecked<'w, 's, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}
unsafe impl<'w, 's, S: Default, Q: HecsQuery> Sync for QueryBorrowChecked<'w, 's, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}

impl<'w, 's, S: Default, Q: HecsQuery> Drop for QueryBorrowChecked<'w, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    #[inline]
    fn drop(&mut self) {
        if self.borrowed {
            for index in self.archetype_access.immutable.ones() {
                Q::Fetch::release(&self.archetypes[index], self.state);
            }

            for index in self.archetype_access.mutable.ones() {
                Q::Fetch::release(&self.archetypes[index], self.state);
            }
        }
    }
}

impl<'q, 'w, 's, S: Default, Q: HecsQuery> IntoIterator for &'q mut QueryBorrowChecked<'w, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type IntoIter = QueryIter<'q, 'w, 's, S, Q>;
    type Item = <Q::Fetch as Fetch<'q>>::Item;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, 'w, 's, S: Default, Q: HecsQuery>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    borrow: &'q mut QueryBorrowChecked<'w, 's, S, Q>,
    archetype_index: usize,
    iter: Option<ChunkIter<'s, S, Q>>,
}

unsafe impl<'q, 'w, 's, S: Default, Q: HecsQuery> Send for QueryIter<'q, 'w, 's, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}
unsafe impl<'q, 'w, 's, S: Default, Q: HecsQuery> Sync for QueryIter<'q, 'w, 's, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}

impl<'q, 'w, 's, S: Default, Q: HecsQuery> Iterator for QueryIter<'q, 'w, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type Item = <Q::Fetch as Fetch<'q>>::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.borrow.archetypes.get(self.archetype_index as usize)?;
                    self.archetype_index += 1;
                    unsafe {
                        self.iter =
                            Q::Fetch::get(archetype, 0, self.borrow.state).map(|fetch| ChunkIter {
                                state: self.borrow.state,
                                fetch,
                                len: archetype.len(),
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

impl<'q, 'w, 's, S: Default, Q: HecsQuery> ExactSizeIterator for QueryIter<'q, 'w, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    fn len(&self) -> usize {
        self.borrow
            .archetypes
            .iter()
            .filter(|&x| Q::Fetch::access(x, self.borrow.state).is_some())
            .map(|x| x.len())
            .sum()
    }
}

struct ChunkIter<'s, S: Default, Q: HecsQuery>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    fetch: Q::Fetch,
    state: &'s S,
    len: usize,
}

impl<'s, S: Default, Q: HecsQuery> ChunkIter<'s, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    #[inline]
    unsafe fn next<'a>(&mut self) -> Option<<Q::Fetch as Fetch<'a>>::Item> {
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
pub struct ParIter<'q, 'w, 's, S: Default, Q: HecsQuery>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    borrow: &'q mut QueryBorrowChecked<'w, 's, S, Q>,
    archetype_index: usize,
    batch_size: usize,
    batch: usize,
}

impl<'q, 'w, 's, S: Default, Q: HecsQuery> ParallelIterator<Batch<'q, 's, S, Q>>
    for ParIter<'q, 'w, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type Item = <Q::Fetch as Fetch<'q>>::Item;

    fn next_batch(&mut self) -> Option<Batch<'q, 's, S, Q>> {
        loop {
            let archetype = self.borrow.archetypes.get(self.archetype_index)?;
            let offset = self.batch_size * self.batch;
            if offset >= archetype.len() {
                self.archetype_index += 1;
                self.batch = 0;
                continue;
            }
            if let Some(fetch) =
                unsafe { Q::Fetch::get(archetype, offset as usize, self.borrow.state) }
            {
                self.batch += 1;
                return Some(Batch {
                    _marker: PhantomData,
                    state: ChunkIter {
                        state: self.borrow.state,
                        fetch,
                        len: self.batch_size.min(archetype.len() - offset),
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
pub struct Batch<'q, 's, S: Default, Q: HecsQuery>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    _marker: PhantomData<&'q ()>,
    state: ChunkIter<'s, S, Q>,
}

impl<'q, 'w, 's, S: Default, Q: HecsQuery> Iterator for Batch<'q, 's, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type Item = <Q::Fetch as Fetch<'q>>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let components = unsafe { self.state.next()? };
        Some(components)
    }
}

unsafe impl<'q, 's, S: Default, Q: HecsQuery> Send for Batch<'q, 's, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}

/// A borrow of a `World` sufficient to execute the query `Q` on a single entity
pub struct QueryOneChecked<'a, 's, S: Default, Q: HecsQuery>
where
    Q::Fetch: for<'b> Fetch<'b, State = S>,
{
    archetype: &'a Archetype,
    state: &'s S,
    index: usize,
    borrowed: bool,
    _marker: PhantomData<Q>,
}

impl<'a, 's, S: Default, Q: HecsQuery> QueryOneChecked<'a, 's, S, Q>
where
    Q::Fetch: for<'b> Fetch<'b, State = S>,
{
    /// Construct a query accessing the entity in `archetype` at `index`
    ///
    /// # Safety
    ///
    /// `index` must be in-bounds for `archetype`
    pub(crate) unsafe fn new(archetype: &'a Archetype, index: usize, state: &'s S) -> Self {
        Self {
            archetype,
            index,
            borrowed: false,
            state,
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
            let mut fetch = Q::Fetch::get(self.archetype, self.index as usize, self.state)?;
            self.borrowed = true;
            Q::Fetch::borrow(self.archetype, self.state);
            Some(fetch.next(self.state))
        }
    }

    /// Transform the query into one that requires a certain component without borrowing it
    ///
    /// See `QueryBorrow::with` for details.
    pub fn with<T: Component>(self) -> QueryOneChecked<'a, 's, S, With<T, Q>>
    where
        <With<T, Q> as HecsQuery>::Fetch: for<'b> Fetch<'b, State = S>,
    {
        self.transform()
    }

    /// Transform the query into one that skips entities having a certain component
    ///
    /// See `QueryBorrow::without` for details.
    pub fn without<T: Component>(self) -> QueryOneChecked<'a, 's, S, Without<T, Q>>
    where
        <Without<T, Q> as HecsQuery>::Fetch: for<'b> Fetch<'b, State = S>,
    {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: HecsQuery>(self) -> QueryOneChecked<'a, 's, S, R>
    where
        R::Fetch: for<'b> Fetch<'b, State = S>,
    {
        QueryOneChecked {
            archetype: self.archetype,
            index: self.index,
            borrowed: self.borrowed,
            state: self.state,
            _marker: PhantomData,
        }
    }
}

impl<'a, 's, S: Default, Q: HecsQuery> Drop for QueryOneChecked<'a, 's, S, Q>
where
    Q::Fetch: for<'b> Fetch<'b, State = S>,
{
    fn drop(&mut self) {
        if self.borrowed {
            Q::Fetch::release(self.archetype, self.state);
        }
    }
}

unsafe impl<'a, 's, S: Default, Q: HecsQuery> Send for QueryOneChecked<'a, 's, S, Q> where
    Q::Fetch: for<'b> Fetch<'b, State = S>
{
}
unsafe impl<'a, 's, S: Default, Q: HecsQuery> Sync for QueryOneChecked<'a, 's, S, Q> where
    Q::Fetch: for<'b> Fetch<'b, State = S>
{
}
