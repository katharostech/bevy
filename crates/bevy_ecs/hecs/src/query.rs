// Copyright 2019 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

// modified by Bevy contributors

use core::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    ptr::{slice_from_raw_parts_mut, NonNull},
};

use crate::{archetype::Archetype, Component, ComponentId, Entity, MissingComponent};

/// A collection of component types to fetch from a `World`
pub trait Query {
    #[doc(hidden)]
    type Fetch: for<'a> Fetch<'a>;
}

/// A fetch that is read only. This should only be implemented for read-only fetches.
pub unsafe trait ReadOnlyFetch {}

/// Streaming iterators over contiguous homogeneous ranges of components
pub trait Fetch<'a>: Sized {
    /// Type of value to be fetched
    type Item;
    /// A type to store state that is needed to `access`, `borrow` or `release`.
    type State: Default;

    /// How this query will access `archetype`, if at all
    fn access(archetype: &Archetype, state: &Self::State) -> Option<Access>;

    /// Acquire dynamic borrows from `archetype`
    fn borrow(archetype: &Archetype, state: &Self::State);
    /// Construct a `Fetch` for `archetype` if it should be traversed
    ///
    /// # Safety
    /// `offset` must be in bounds of `archetype`
    unsafe fn get(archetype: &'a Archetype, offset: usize, state: &Self::State) -> Option<Self>;
    /// Release dynamic borrows acquired by `borrow`
    fn release(archetype: &Archetype, state: &Self::State);

    /// if this returns true, the current item will be skipped during iteration
    ///
    /// # Safety
    /// shouldn't be called if there is no current item
    unsafe fn should_skip(&self, _state: &Self::State) -> bool {
        false
    }

    /// Access the next item in this archetype without bounds checking
    ///
    /// # Safety
    /// - Must only be called after `borrow`
    /// - `release` must not be called while `'a` is still live
    /// - Bounds-checking must be performed externally
    /// - Any resulting borrows must be legal (e.g. no &mut to something another iterator might access)
    unsafe fn next(&mut self, state: &Self::State) -> Self::Item;
}

/// Type of access a `Query` may have to an `Archetype`
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum Access {
    /// Read entity IDs only, no components
    Iterate,
    /// Read components
    Read,
    /// Read and write components
    Write,
}

/// The information needed to access a dynamic component, one that's info is determined at runtime
/// instead of compile time
#[derive(Debug, Copy, Clone)]
pub struct DynamicComponentInfo {
    id: ComponentId,
    size: usize,
}

/// The requested access to a dynamic component
#[derive(Debug, Copy, Clone)]
pub struct DynamicComponentAccess {
    info: DynamicComponentInfo,
    access: Access,
}

/// A dynamically constructable component query
#[derive(Debug, Clone)]
pub struct DynamicComponentQuery([Option<DynamicComponentAccess>; 64]);

impl Default for DynamicComponentQuery {
    fn default() -> Self {
        DynamicComponentQuery([None; 64])
    }
}

impl std::ops::Deref for DynamicComponentQuery {
    type Target = [Option<DynamicComponentAccess>; 64];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for DynamicComponentQuery {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// A query that can be constructed at runtime
pub struct RuntimeQuery;

impl Query for RuntimeQuery {
    type Fetch = DynamicFetch;
}

/// A [`Fetch`] implementation for dynamic components
#[derive(Debug)]
pub struct DynamicFetch {
    datas: [Option<NonNull<[u8]>>; 64],
}

impl Default for DynamicFetch {
    fn default() -> Self {
        DynamicFetch { datas: [None; 64] }
    }
}

impl<'a> Fetch<'a> for DynamicFetch {
    type Item = [Option<&'a [u8]>; 64];
    type State = DynamicComponentQuery;

    fn access(archetype: &Archetype, state: &Self::State) -> Option<Access> {
        let mut access = None;

        for component_access in state.iter().filter_map(|x| x.as_ref()) {
            if archetype.has_component(component_access.info.id) {
                access = access.map_or(Some(component_access.access), |access| {
                    if access < component_access.access {
                        Some(component_access.access)
                    } else {
                        Some(access)
                    }
                });
            }
        }

        access
    }

    fn borrow(archetype: &Archetype, state: &Self::State) {
        for component_access in state.iter().filter_map(|&x| x) {
            archetype.borrow_component(component_access.info.id);
        }
    }

    fn release(archetype: &Archetype, state: &Self::State) {
        for component_access in state.iter().filter_map(|&x| x) {
            archetype.release_component(component_access.info.id);
        }
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, state: &Self::State) -> Option<Self> {
        let mut fetch = Self { datas: [None; 64] };

        let mut matches_any = false;
        for (component_index, component_access) in state
            .iter()
            .enumerate()
            .filter_map(|(i, &x)| x.map(|y| (i, y)))
        {
            let ptr =
                archetype.get_dynamic(component_access.info.id, component_access.info.size, offset);

            if ptr.is_some() {
                matches_any = true
            }

            fetch.datas[component_index] = ptr.map(|x| {
                NonNull::new_unchecked(slice_from_raw_parts_mut(
                    x.as_ptr(),
                    component_access.info.size,
                ))
            });
        }

        if matches_any {
            Some(fetch)
        } else {
            None
        }
    }

    unsafe fn next(&mut self, state: &Self::State) -> Self::Item {
        let mut components = [None; 64];

        for (component_index, component_access) in state
            .iter()
            .enumerate()
            .filter_map(|(i, &x)| x.map(|y| (i, y)))
        {
            if let Some(nonnull) = &mut self.datas[component_index] {
                components[component_index] = {
                    let x = nonnull.as_ptr();
                    *nonnull = NonNull::new_unchecked(slice_from_raw_parts_mut(
                        (x as *mut u8).add(component_access.info.size),
                        component_access.info.size,
                    ));
                    Some(&*x)
                };
            }
        }

        components
    }

    unsafe fn should_skip(&self, _state: &Self::State) -> bool {
        false
    }
}

#[derive(Copy, Clone, Debug)]
pub struct EntityFetch(NonNull<Entity>);
unsafe impl ReadOnlyFetch for EntityFetch {}

impl Query for Entity {
    type Fetch = EntityFetch;
}

impl<'a> Fetch<'a> for EntityFetch {
    type Item = Entity;
    type State = ();

    #[inline]
    fn access(_archetype: &Archetype, _: &Self::State) -> Option<Access> {
        Some(Access::Iterate)
    }

    #[inline]
    fn borrow(_archetype: &Archetype, _: &Self::State) {}

    #[inline]
    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        Some(EntityFetch(NonNull::new_unchecked(
            archetype.entities().as_ptr().add(offset),
        )))
    }

    #[inline]
    fn release(_archetype: &Archetype, _: &Self::State) {}

    #[inline]
    unsafe fn next(&mut self, _: &Self::State) -> Self::Item {
        let id = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(id.add(1));
        *id
    }
}

impl<'a, T: Component> Query for &'a T {
    type Fetch = FetchRead<T>;
}

#[doc(hidden)]
pub struct FetchRead<T>(NonNull<T>);

unsafe impl<T> ReadOnlyFetch for FetchRead<T> {}

impl<'a, T: Component> Fetch<'a> for FetchRead<T> {
    type Item = &'a T;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Read)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        archetype.borrow::<T>();
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        archetype
            .get::<T>()
            .map(|x| Self(NonNull::new_unchecked(x.as_ptr().add(offset))))
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        archetype.release::<T>();
    }

    #[inline]
    unsafe fn next(&mut self, _: &Self::State) -> &'a T {
        let x = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(x.add(1));
        &*x
    }
}

impl<'a, T: Component> Query for &'a mut T {
    type Fetch = FetchMut<T>;
}

impl<T: Query> Query for Option<T> {
    type Fetch = TryFetch<T::Fetch>;
}

/// Unique borrow of an entity's component
pub struct Mut<'a, T: Component> {
    pub(crate) value: &'a mut T,
    pub(crate) mutated: &'a mut bool,
}

impl<'a, T: Component> Mut<'a, T> {
    /// Creates a new mutable reference to a component. This is unsafe because the index bounds are not checked.
    ///
    /// # Safety
    /// This doesn't check the bounds of index in archetype
    pub unsafe fn new(archetype: &'a Archetype, index: usize) -> Result<Self, MissingComponent> {
        let (target, type_state) = archetype
            .get_with_type_state::<T>()
            .ok_or_else(MissingComponent::new::<T>)?;
        Ok(Self {
            value: &mut *target.as_ptr().add(index),
            mutated: &mut *type_state.mutated().as_ptr().add(index),
        })
    }
}

unsafe impl<T: Component> Send for Mut<'_, T> {}
unsafe impl<T: Component> Sync for Mut<'_, T> {}

impl<'a, T: Component> Deref for Mut<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'a, T: Component> DerefMut for Mut<'a, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        *self.mutated = true;
        self.value
    }
}

impl<'a, T: Component + core::fmt::Debug> core::fmt::Debug for Mut<'a, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.value.fmt(f)
    }
}

impl<'a, T: Component> Query for Mut<'a, T> {
    type Fetch = FetchMut<T>;
}
#[doc(hidden)]
pub struct FetchMut<T>(NonNull<T>, NonNull<bool>);

impl<'a, T: Component> Fetch<'a> for FetchMut<T> {
    type Item = Mut<'a, T>;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Write)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        archetype.borrow_mut::<T>();
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        archetype
            .get_with_type_state::<T>()
            .map(|(components, type_state)| {
                Self(
                    NonNull::new_unchecked(components.as_ptr().add(offset)),
                    NonNull::new_unchecked(type_state.mutated().as_ptr().add(offset)),
                )
            })
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        archetype.release_mut::<T>();
    }

    #[inline]
    unsafe fn next(&mut self, _: &Self::State) -> Mut<'a, T> {
        let component = self.0.as_ptr();
        let mutated = self.1.as_ptr();
        self.0 = NonNull::new_unchecked(component.add(1));
        self.1 = NonNull::new_unchecked(mutated.add(1));
        Mut {
            value: &mut *component,
            mutated: &mut *mutated,
        }
    }
}

macro_rules! impl_or_query {
    ( $( $T:ident ),+ ) => {
        impl<$( $T: Query ),+> Query for Or<($( $T ),+)> {
            type Fetch = FetchOr<($( $T::Fetch ),+)>;
        }

        impl<'a, $( $T: Fetch<'a> ),+> Fetch<'a> for FetchOr<($( $T ),+)> {
            type Item = ($( $T::Item ),+);
            type State = ();

            fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
                let mut max_access = None;
                $(
                max_access = max_access.max($T::access(archetype, &Default::default()));
                )+
                max_access
            }

            fn borrow(archetype: &Archetype, _: &Self::State) {
                $(
                    $T::borrow(archetype, &Default::default());
                 )+
            }

            unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
                Some(Self(( $( $T::get(archetype, offset, &Default::default())?),+ )))
            }

            fn release(archetype: &Archetype, _: &Self::State) {
                $(
                    $T::release(archetype, &Default::default());
                 )+
            }

            #[allow(non_snake_case)]
            unsafe fn next(&mut self, _: &Self::State) -> Self::Item {
                let ($( $T ),+) = &mut self.0;
                ($( $T.next(&Default::default()) ),+)
            }

             #[allow(non_snake_case)]
            unsafe fn should_skip(&self, _: &Self::State) -> bool {
                let ($( $T ),+) = &self.0;
                true $( && $T.should_skip(&Default::default()) )+
            }
        }
    };
}

impl_or_query!(Q1, Q2);
impl_or_query!(Q1, Q2, Q3);
impl_or_query!(Q1, Q2, Q3, Q4);
impl_or_query!(Q1, Q2, Q3, Q4, Q5);
impl_or_query!(Q1, Q2, Q3, Q4, Q5, Q6);
impl_or_query!(Q1, Q2, Q3, Q4, Q5, Q6, Q7);
impl_or_query!(Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8);
impl_or_query!(Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9);
impl_or_query!(Q1, Q2, Q3, Q4, Q5, Q6, Q7, Q8, Q9, Q10);

/// Query transformer performing a logical or on a pair of queries Intended to be used on Mutated or
/// Changed queries.
/// # Example
/// ```
/// # use bevy_hecs::*;
/// let mut world = World::new();
/// world.spawn((123, true, 1., Some(1)));
/// world.spawn((456, false, 2., Some(0)));
/// for mut b in world.query_mut::<Mut<i32>>().iter().skip(1).take(1) {
///     *b += 1;
/// }
/// let components = world
///     .query_mut::<Or<(Mutated<bool>, Mutated<i32>, Mutated<f64>, Mutated<Option<i32>>)>>()
///     .iter()
///     .map(|(b, i, f, o)| (*b, *i))
///     .collect::<Vec<_>>();
/// assert_eq!(components, &[(false, 457)]);
/// ```
pub struct Or<T>(PhantomData<T>);
//pub struct Or<Q1, Q2, Q3>(PhantomData<(Q1, Q2, Q3)>);

#[doc(hidden)]
pub struct FetchOr<T>(T);

/// Query transformer that retrieves components of type `T` that have been mutated since the start
/// of the frame. Added components do not count as mutated.
pub struct Mutated<'a, T> {
    value: &'a T,
}

impl<'a, T: Component> Deref for Mutated<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'a, T: Component> Query for Mutated<'a, T> {
    type Fetch = FetchMutated<T>;
}

#[doc(hidden)]
pub struct FetchMutated<T>(NonNull<T>, NonNull<bool>);

impl<'a, T: Component> Fetch<'a> for FetchMutated<T> {
    type Item = Mutated<'a, T>;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Read)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        archetype.borrow::<T>();
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        archetype
            .get_with_type_state::<T>()
            .map(|(components, type_state)| {
                Self(
                    NonNull::new_unchecked(components.as_ptr().add(offset)),
                    NonNull::new_unchecked(type_state.mutated().as_ptr().add(offset)),
                )
            })
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        archetype.release::<T>();
    }

    unsafe fn should_skip(&self, _: &Self::State) -> bool {
        // skip if the current item wasn't mutated
        !*self.1.as_ref()
    }

    #[inline]
    unsafe fn next(&mut self, _: &Self::State) -> Self::Item {
        self.1 = NonNull::new_unchecked(self.1.as_ptr().add(1));
        let value = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(value.add(1));
        Mutated { value: &*value }
    }
}

/// Query transformer that retrieves components of type `T` that have been added since the start of
/// the frame.
pub struct Added<'a, T> {
    value: &'a T,
}

impl<'a, T: Component> Deref for Added<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'a, T: Component> Query for Added<'a, T> {
    type Fetch = FetchAdded<T>;
}

#[doc(hidden)]
pub struct FetchAdded<T>(NonNull<T>, NonNull<bool>);
unsafe impl<T> ReadOnlyFetch for FetchAdded<T> {}

impl<'a, T: Component> Fetch<'a> for FetchAdded<T> {
    type Item = Added<'a, T>;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Read)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        archetype.borrow::<T>();
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        archetype
            .get_with_type_state::<T>()
            .map(|(components, type_state)| {
                Self(
                    NonNull::new_unchecked(components.as_ptr().add(offset)),
                    NonNull::new_unchecked(type_state.added().as_ptr().add(offset)),
                )
            })
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        archetype.release::<T>();
    }

    unsafe fn should_skip(&self, _: &Self::State) -> bool {
        // skip if the current item wasn't added
        !*self.1.as_ref()
    }

    #[inline]
    unsafe fn next(&mut self, _: &Self::State) -> Self::Item {
        self.1 = NonNull::new_unchecked(self.1.as_ptr().add(1));
        let value = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(value.add(1));
        Added { value: &*value }
    }
}

/// Query transformer that retrieves components of type `T` that have either been mutated or added
/// since the start of the frame.
pub struct Changed<'a, T> {
    value: &'a T,
}

impl<'a, T: Component> Deref for Changed<'a, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        self.value
    }
}

impl<'a, T: Component> Query for Changed<'a, T> {
    type Fetch = FetchChanged<T>;
}

#[doc(hidden)]
pub struct FetchChanged<T>(NonNull<T>, NonNull<bool>, NonNull<bool>);
unsafe impl<T> ReadOnlyFetch for FetchChanged<T> {}

impl<'a, T: Component> Fetch<'a> for FetchChanged<T> {
    type Item = Changed<'a, T>;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        if archetype.has::<T>() {
            Some(Access::Read)
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        archetype.borrow::<T>();
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        archetype
            .get_with_type_state::<T>()
            .map(|(components, type_state)| {
                Self(
                    NonNull::new_unchecked(components.as_ptr().add(offset)),
                    NonNull::new_unchecked(type_state.added().as_ptr().add(offset)),
                    NonNull::new_unchecked(type_state.mutated().as_ptr().add(offset)),
                )
            })
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        archetype.release::<T>();
    }

    unsafe fn should_skip(&self, _: &Self::State) -> bool {
        // skip if the current item wasn't added or mutated
        !*self.1.as_ref() && !self.2.as_ref()
    }

    #[inline]
    unsafe fn next(&mut self, _: &Self::State) -> Self::Item {
        self.1 = NonNull::new_unchecked(self.1.as_ptr().add(1));
        self.2 = NonNull::new_unchecked(self.2.as_ptr().add(1));
        let value = self.0.as_ptr();
        self.0 = NonNull::new_unchecked(value.add(1));
        Changed { value: &*value }
    }
}

#[doc(hidden)]
pub struct TryFetch<T>(Option<T>);
unsafe impl<T> ReadOnlyFetch for TryFetch<T> where T: ReadOnlyFetch {}

impl<'a, T: Fetch<'a>> Fetch<'a> for TryFetch<T> {
    type Item = Option<T::Item>;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        Some(T::access(archetype, &Default::default()).unwrap_or(Access::Iterate))
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        T::borrow(archetype, &Default::default())
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        Some(Self(T::get(archetype, offset, &Default::default())))
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        T::release(archetype, &Default::default())
    }

    unsafe fn next(&mut self, _: &Self::State) -> Option<T::Item> {
        Some(self.0.as_mut()?.next(&Default::default()))
    }

    unsafe fn should_skip(&self, _: &Self::State) -> bool {
        self.0
            .as_ref()
            .map_or(false, |fetch| fetch.should_skip(&Default::default()))
    }
}

/// Query transformer skipping entities that have a `T` component
///
/// See also `QueryBorrow::without`.
///
/// # Example
/// ```
/// # use bevy_hecs::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let entities = world.query::<Without<bool, (Entity, &i32)>>()
///     .iter()
///     .map(|(e, &i)| (e, i))
///     .collect::<Vec<_>>();
/// assert_eq!(entities, &[(c, 42)]);
/// ```
pub struct Without<T, Q>(PhantomData<(Q, fn(T))>);

impl<T: Component, Q: Query> Query for Without<T, Q> {
    type Fetch = FetchWithout<T, Q::Fetch>;
}

#[doc(hidden)]
pub struct FetchWithout<T, F>(F, PhantomData<fn(T)>);
unsafe impl<'a, T: Component, F: Fetch<'a>> ReadOnlyFetch for FetchWithout<T, F> where
    F: ReadOnlyFetch
{
}

impl<'a, T: Component, F: Fetch<'a>> Fetch<'a> for FetchWithout<T, F> {
    type Item = F::Item;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        if archetype.has::<T>() {
            None
        } else {
            F::access(archetype, &Default::default())
        }
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        F::borrow(archetype, &Default::default())
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        if archetype.has::<T>() {
            return None;
        }
        Some(Self(
            F::get(archetype, offset, &Default::default())?,
            PhantomData,
        ))
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        F::release(archetype, &Default::default())
    }

    unsafe fn next(&mut self, _: &Self::State) -> F::Item {
        self.0.next(&Default::default())
    }

    unsafe fn should_skip(&self, _: &Self::State) -> bool {
        self.0.should_skip(&Default::default())
    }
}

/// Query transformer skipping entities that do not have a `T` component
///
/// See also `QueryBorrow::with`.
///
/// # Example
/// ```
/// # use bevy_hecs::*;
/// let mut world = World::new();
/// let a = world.spawn((123, true, "abc"));
/// let b = world.spawn((456, false));
/// let c = world.spawn((42, "def"));
/// let entities = world.query::<With<bool, (Entity, &i32)>>()
///     .iter()
///     .map(|(e, &i)| (e, i))
///     .collect::<Vec<_>>();
/// assert_eq!(entities.len(), 2);
/// assert!(entities.contains(&(a, 123)));
/// assert!(entities.contains(&(b, 456)));
/// ```
pub struct With<T, Q>(PhantomData<(Q, fn(T))>);

impl<T: Component, Q: Query> Query for With<T, Q> {
    type Fetch = FetchWith<T, Q::Fetch>;
}

#[doc(hidden)]
pub struct FetchWith<T, F>(F, PhantomData<fn(T)>);
unsafe impl<'a, T: Component, F: Fetch<'a>> ReadOnlyFetch for FetchWith<T, F> where F: ReadOnlyFetch {}

impl<'a, T: Component, F: Fetch<'a>> Fetch<'a> for FetchWith<T, F> {
    type Item = F::Item;
    type State = ();

    fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
        if archetype.has::<T>() {
            F::access(archetype, &Default::default())
        } else {
            None
        }
    }

    fn borrow(archetype: &Archetype, _: &Self::State) {
        F::borrow(archetype, &Default::default())
    }

    unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
        if !archetype.has::<T>() {
            return None;
        }
        Some(Self(
            F::get(archetype, offset, &Default::default())?,
            PhantomData,
        ))
    }

    fn release(archetype: &Archetype, _: &Self::State) {
        F::release(archetype, &Default::default())
    }

    unsafe fn next(&mut self, _: &Self::State) -> F::Item {
        self.0.next(&Default::default())
    }

    unsafe fn should_skip(&self, _: &Self::State) -> bool {
        self.0.should_skip(&Default::default())
    }
}

/// A borrow of a `World` sufficient to execute the query `Q`
///
/// Note that borrows are not released until this object is dropped.
pub struct QueryBorrow<'w, S: Default, Q: Query>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    archetypes: &'w [Archetype],
    borrowed: bool,
    state: S,
    _marker: PhantomData<Q>,
}

impl<'w, S: Default, Q: Query> QueryBorrow<'w, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    pub(crate) fn new(archetypes: &'w [Archetype], state: S) -> Self {
        Self {
            archetypes,
            borrowed: false,
            state,
            _marker: PhantomData,
        }
    }

    /// Execute the query
    ///
    /// Must be called only once per query.
    pub fn iter<'q>(&'q mut self) -> QueryIter<'q, 'w, S, Q> {
        self.borrow();
        QueryIter {
            archetypes: self.archetypes,
            state: &self.state,
            archetype_index: 0,
            iter: None,
        }
    }

    /// Like `iter`, but returns child iterators of at most `batch_size` elements
    ///
    /// Useful for distributing work over a threadpool.
    pub fn iter_batched<'q>(&'q mut self, batch_size: usize) -> BatchedIter<'q, 'w, S, Q> {
        self.borrow();
        BatchedIter {
            archetypes: self.archetypes,
            state: &self.state,
            archetype_index: 0,
            batch_size,
            batch: 0,
            _marker: PhantomData,
        }
    }

    fn borrow(&mut self) {
        if self.borrowed {
            panic!(
                "called QueryBorrow::iter twice on the same borrow; construct a new query instead"
            );
        }

        self.borrowed = true;
    }

    /// Transform the query into one that requires a certain component without borrowing it
    ///
    /// This can be useful when the component needs to be borrowed elsewhere and it isn't necessary
    /// for the iterator to expose its data directly.
    ///
    /// Equivalent to using a query type wrapped in `With`.
    ///
    /// # Example
    /// ```
    /// # use bevy_hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let entities = world.query::<(Entity, &i32)>()
    ///     .with::<bool>()
    ///     .iter()
    ///     .map(|(e, &i)| (e, i)) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert!(entities.contains(&(a, 123)));
    /// assert!(entities.contains(&(b, 456)));
    /// ```
    pub fn with<T: Component>(self) -> QueryBorrow<'w, S, With<T, Q>>
    where
        <With<T, Q> as Query>::Fetch: for<'a> Fetch<'a, State = S>,
    {
        self.transform()
    }

    /// Transform the query into one that skips entities having a certain component
    ///
    /// Equivalent to using a query type wrapped in `Without`.
    ///
    /// # Example
    /// ```
    /// # use bevy_hecs::*;
    /// let mut world = World::new();
    /// let a = world.spawn((123, true, "abc"));
    /// let b = world.spawn((456, false));
    /// let c = world.spawn((42, "def"));
    /// let entities = world.query::<(Entity, &i32)>()
    ///     .without::<bool>()
    ///     .iter()
    ///     .map(|(e, &i)| (e, i)) // Copy out of the world
    ///     .collect::<Vec<_>>();
    /// assert_eq!(entities, &[(c, 42)]);
    /// ```
    pub fn without<T: Component>(self) -> QueryBorrow<'w, S, Without<T, Q>>
    where
        <Without<T, Q> as Query>::Fetch: for<'a> Fetch<'a, State = S>,
    {
        self.transform()
    }

    /// Helper to change the type of the query
    fn transform<R: Query>(mut self) -> QueryBorrow<'w, S, R>
    where
        R::Fetch: for<'a> Fetch<'a, State = S>,
    {
        let borrow = QueryBorrow {
            archetypes: self.archetypes,
            borrowed: self.borrowed,
            state: self.state,
            _marker: PhantomData,
        };

        self.borrowed = false;
        borrow
    }
}

unsafe impl<'w, 's, S: Default, Q: Query> Send for QueryBorrow<'w, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}
unsafe impl<'w, 's, S: Default, Q: Query> Sync for QueryBorrow<'w, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}

impl<'q, 'w, S: Default, Q: Query> IntoIterator for &'q mut QueryBorrow<'w, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type IntoIter = QueryIter<'q, 'w, S, Q>;
    type Item = <Q::Fetch as Fetch<'q>>::Item;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Iterator over the set of entities with the components in `Q`
pub struct QueryIter<'q, 'w, S: Default, Q: Query>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    archetypes: &'w [Archetype],
    state: &'q S,
    archetype_index: usize,
    iter: Option<ChunkIter<'q, S, Q>>,
}

unsafe impl<'q, 'w, 's, S: Default, Q: Query> Send for QueryIter<'q, 'w, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}
unsafe impl<'q, 'w, 's, S: Default, Q: Query> Sync for QueryIter<'q, 'w, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}

impl<'q, 'w, S: Default, Q: Query> Iterator for QueryIter<'q, 'w, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type Item = <Q::Fetch as Fetch<'q>>::Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.iter {
                None => {
                    let archetype = self.archetypes.get(self.archetype_index)?;
                    self.archetype_index += 1;
                    unsafe {
                        self.iter =
                            Q::Fetch::get(archetype, 0, self.state).map(|fetch| ChunkIter {
                                state: self.state,
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

impl<'q, 'w, 's, S: Default, Q: Query> ExactSizeIterator for QueryIter<'q, 'w, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    fn len(&self) -> usize {
        self.archetypes
            .iter()
            .filter(|&x| Q::Fetch::access(x, self.state).is_some())
            .map(|x| x.len())
            .sum()
    }
}

struct ChunkIter<'s, S: Default, Q: Query>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    fetch: Q::Fetch,
    state: &'s S,
    len: usize,
}

impl<'s, S: Default, Q: Query> ChunkIter<'s, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
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
pub struct BatchedIter<'q, 'w, S: Default, Q: Query>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    archetypes: &'w [Archetype],
    state: &'q S,
    archetype_index: usize,
    batch_size: usize,
    batch: usize,
    _marker: PhantomData<Q>,
}

unsafe impl<'q, 'w, 's, S: Default, Q: Query> Send for BatchedIter<'q, 'w, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}
unsafe impl<'q, 'w, 's, S: Default, Q: Query> Sync for BatchedIter<'q, 'w, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}

impl<'q, 'w, S: Default, Q: Query> Iterator for BatchedIter<'q, 'w, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type Item = Batch<'q, S, Q>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let archetype = self.archetypes.get(self.archetype_index)?;
            let offset = self.batch_size * self.batch;
            if offset >= archetype.len() {
                self.archetype_index += 1;
                self.batch = 0;
                continue;
            }
            if let Some(fetch) = unsafe { Q::Fetch::get(archetype, offset, self.state) } {
                self.batch += 1;
                return Some(Batch {
                    _marker: PhantomData,
                    state: ChunkIter {
                        state: self.state,
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

/// A sequence of entities yielded by `BatchedIter`
pub struct Batch<'q, S: Default, Q: Query>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    _marker: PhantomData<&'q ()>,
    state: ChunkIter<'q, S, Q>,
}

impl<'q, 'w, 's, S: Default, Q: Query> Iterator for Batch<'q, S, Q>
where
    Q::Fetch: for<'a> Fetch<'a, State = S>,
{
    type Item = <Q::Fetch as Fetch<'q>>::Item;

    fn next(&mut self) -> Option<Self::Item> {
        let components = unsafe { self.state.next()? };
        Some(components)
    }
}

unsafe impl<'q, 's, S: Default, Q: Query> Send for Batch<'q, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}
unsafe impl<'q, 's, S: Default, Q: Query> Sync for Batch<'q, S, Q> where
    Q::Fetch: for<'a> Fetch<'a, State = S>
{
}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<'a, $($name: Fetch<'a>),*> Fetch<'a> for ($($name,)*) {
            type Item = ($($name::Item,)*);
            type State = ();

            #[allow(unused_variables, unused_mut)]
            fn access(archetype: &Archetype, _: &Self::State) -> Option<Access> {
                let mut access = Access::Iterate;
                $(
                    access = access.max($name::access(archetype, &Default::default())?);
                )*
                Some(access)
            }

            #[allow(unused_variables)]
            fn borrow(archetype: &Archetype, _: &Self::State) {
                $($name::borrow(archetype, &Default::default());)*
            }
            #[allow(unused_variables)]
            unsafe fn get(archetype: &'a Archetype, offset: usize, _: &Self::State) -> Option<Self> {
                Some(($($name::get(archetype, offset, &Default::default())?,)*))
            }
            #[allow(unused_variables)]
            fn release(archetype: &Archetype, _: &Self::State) {
                $($name::release(archetype, &Default::default());)*
            }

            #[allow(unused_variables)]
            unsafe fn next(&mut self, _: &Self::State) -> Self::Item {
                #[allow(non_snake_case)]
                let ($($name,)*) = self;
                ($($name.next(&Default::default()),)*)
            }

            unsafe fn should_skip(&self, _: &Self::State) -> bool {
                #[allow(non_snake_case)]
                let ($($name,)*) = self;
                $($name.should_skip(&Default::default())||)* false
            }
        }

        impl<$($name: Query),*> Query for ($($name,)*) {
            type Fetch = ($($name::Fetch,)*);
        }

        unsafe impl<$($name: ReadOnlyFetch),*> ReadOnlyFetch for ($($name,)*) {}
    };
}

smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);

#[cfg(test)]
mod tests {
    use crate::{Entity, Mut, Mutated, World};
    use std::{vec, vec::Vec};

    use super::*;

    #[test]
    fn access_order() {
        assert!(Access::Write > Access::Read);
        assert!(Access::Read > Access::Iterate);
        assert!(Some(Access::Iterate) > None);
    }

    struct A(usize);
    struct B(usize);
    struct C;

    #[test]
    fn added_queries() {
        let mut world = World::default();
        let e1 = world.spawn((A(0),));

        fn get_added<Com: Component>(world: &World) -> Vec<Entity> {
            world
                .query::<(Added<Com>, Entity)>()
                .iter()
                .map(|(_added, e)| e)
                .collect::<Vec<Entity>>()
        };

        assert_eq!(get_added::<A>(&world), vec![e1]);
        world.insert(e1, (B(0),)).unwrap();
        assert_eq!(get_added::<A>(&world), vec![e1]);
        assert_eq!(get_added::<B>(&world), vec![e1]);

        world.clear_trackers();
        assert!(get_added::<A>(&world).is_empty());
        let e2 = world.spawn((A(1), B(1)));
        assert_eq!(get_added::<A>(&world), vec![e2]);
        assert_eq!(get_added::<B>(&world), vec![e2]);

        let added = world
            .query::<(Entity, Added<A>, Added<B>)>()
            .iter()
            .map(|a| a.0)
            .collect::<Vec<Entity>>();
        assert_eq!(added, vec![e2]);
    }

    #[test]
    fn mutated_trackers() {
        let mut world = World::default();
        let e1 = world.spawn((A(0), B(0)));
        let e2 = world.spawn((A(0), B(0)));
        let e3 = world.spawn((A(0), B(0)));
        world.spawn((A(0), B));

        for (i, mut a) in world.query_mut::<Mut<A>>().iter().enumerate() {
            if i % 2 == 0 {
                a.0 += 1;
            }
        }

        fn get_changed_a(world: &mut World) -> Vec<Entity> {
            world
                .query_mut::<(Mutated<A>, Entity)>()
                .iter()
                .map(|(_a, e)| e)
                .collect::<Vec<Entity>>()
        };

        assert_eq!(get_changed_a(&mut world), vec![e1, e3]);

        // ensure changing an entity's archetypes also moves its mutated state
        world.insert(e1, (C,)).unwrap();

        assert_eq!(get_changed_a(&mut world), vec![e3, e1], "changed entities list should not change (although the order will due to archetype moves)");

        // spawning a new A entity should not change existing mutated state
        world.insert(e1, (A(0), B)).unwrap();
        assert_eq!(
            get_changed_a(&mut world),
            vec![e3, e1],
            "changed entities list should not change"
        );

        // removing an unchanged entity should not change mutated state
        world.despawn(e2).unwrap();
        assert_eq!(
            get_changed_a(&mut world),
            vec![e3, e1],
            "changed entities list should not change"
        );

        // removing a changed entity should remove it from enumeration
        world.despawn(e1).unwrap();
        assert_eq!(
            get_changed_a(&mut world),
            vec![e3],
            "e1 should no longer be returned"
        );

        world.clear_trackers();

        assert!(world
            .query_mut::<(Mutated<A>, Entity)>()
            .iter()
            .map(|(_a, e)| e)
            .collect::<Vec<Entity>>()
            .is_empty());
    }

    #[test]
    fn multiple_mutated_query() {
        let mut world = World::default();
        world.spawn((A(0), B(0)));
        let e2 = world.spawn((A(0), B(0)));
        world.spawn((A(0), B(0)));

        for mut a in world.query_mut::<Mut<A>>().iter() {
            a.0 += 1;
        }

        for mut b in world.query_mut::<Mut<B>>().iter().skip(1).take(1) {
            b.0 += 1;
        }

        let a_b_changed = world
            .query_mut::<(Mutated<A>, Mutated<B>, Entity)>()
            .iter()
            .map(|(_a, _b, e)| e)
            .collect::<Vec<Entity>>();
        assert_eq!(a_b_changed, vec![e2]);
    }

    #[test]
    fn or_mutated_query() {
        let mut world = World::default();
        let e1 = world.spawn((A(0), B(0)));
        let e2 = world.spawn((A(0), B(0)));
        let e3 = world.spawn((A(0), B(0)));
        let _e4 = world.spawn((A(0), B(0)));

        // Mutate A in entities e1 and e2
        for mut a in world.query_mut::<Mut<A>>().iter().take(2) {
            a.0 += 1;
        }
        // Mutate B in entities e2 and e3
        for mut b in world.query_mut::<Mut<B>>().iter().skip(1).take(2) {
            b.0 += 1;
        }

        let a_b_changed = world
            .query_mut::<(Or<(Mutated<A>, Mutated<B>)>, Entity)>()
            .iter()
            .map(|((_a, _b), e)| e)
            .collect::<Vec<Entity>>();
        // e1 has mutated A, e3 has mutated B, e2 has mutated A and B, _e4 has no mutated component
        assert_eq!(a_b_changed, vec![e1, e2, e3]);
    }

    #[test]
    fn changed_query() {
        let mut world = World::default();
        let e1 = world.spawn((A(0), B(0)));

        fn get_changed(world: &World) -> Vec<Entity> {
            world
                .query::<(Changed<A>, Entity)>()
                .iter()
                .map(|(_a, e)| e)
                .collect::<Vec<Entity>>()
        };
        assert_eq!(get_changed(&world), vec![e1]);
        world.clear_trackers();
        assert_eq!(get_changed(&world), vec![]);
        *world.get_mut(e1).unwrap() = A(1);
        assert_eq!(get_changed(&world), vec![e1]);
    }
}
