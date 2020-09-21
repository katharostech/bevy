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

use crate::{
    alloc::{vec, vec::Vec},
    Archetype,
};
use core::{
    any::{type_name, TypeId},
    fmt, mem,
};

use crate::{archetype::TypeInfo, Component};

/// A dynamically typed collection of components
pub trait DynamicBundle {
    /// Invoke a callback on the fields' type IDs, sorted by descending alignment then id
    #[doc(hidden)]
    fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T;
    /// Obtain the fields' TypeInfos, sorted by descending alignment then id
    #[doc(hidden)]
    fn type_info(&self) -> Vec<TypeInfo>;
    /// Allow a callback to move all components out of the bundle
    ///
    /// Must invoke `f` only with a valid pointer, its type, and the pointee's size. A `false`
    /// return value indicates that the value was not moved and should be dropped.
    #[doc(hidden)]
    fn put(self, archetype: &mut Archetype);
}

/// A statically typed collection of components
pub trait Bundle: DynamicBundle {
    #[doc(hidden)]
    fn with_static_ids<T>(f: impl FnOnce(&[TypeId]) -> T) -> T;

    /// Obtain the fields' TypeInfos, sorted by descending alignment then id
    #[doc(hidden)]
    fn static_type_info() -> Vec<TypeInfo>;

    /// Construct `Self` by moving components out of pointers fetched by `f`
    ///
    /// # Safety
    ///
    /// `f` must produce pointers to the expected fields. The implementation must not read from any
    /// pointers if any call to `f` returns `None`.
    #[doc(hidden)]
    unsafe fn get(archetype: &Archetype, index: usize) -> Result<Self, MissingComponent> where Self: Sized;
}

/// Error indicating that an entity did not have a required component
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MissingComponent(&'static str);

impl MissingComponent {
    /// Construct an error representing a missing `T`
    pub fn new<T: Component>() -> Self {
        Self(type_name::<T>())
    }
}

impl fmt::Display for MissingComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "missing {} component", self.0)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for MissingComponent {}

macro_rules! tuple_impl {
    ($($name: ident),*) => {
        impl<$($name: Component),*> DynamicBundle for ($($name,)*) {
            fn with_ids<T>(&self, f: impl FnOnce(&[TypeId]) -> T) -> T {
                Self::with_static_ids(f)
            }

            fn type_info(&self) -> Vec<TypeInfo> {
                Self::static_type_info()
            }

            #[allow(unused_variables, unused_mut)]
            fn put(self, archetype: &mut Archetype) {
                #[allow(non_snake_case)]
                let ($(mut $name,)*) = self;
                $(
                    archetype.insert($name);
                )*
            }
        }

        impl<$($name: Component),*> Bundle for ($($name,)*) {
            fn with_static_ids<T>(f: impl FnOnce(&[TypeId]) -> T) -> T {
                const N: usize = count!($($name),*);
                let mut xs: [(usize, TypeId); N] = [$((mem::align_of::<$name>(), TypeId::of::<$name>())),*];
                xs.sort_unstable_by(|x, y| x.0.cmp(&y.0).reverse().then(x.1.cmp(&y.1)));
                let mut ids = [TypeId::of::<()>(); N];
                for (slot, &(_, id)) in ids.iter_mut().zip(xs.iter()) {
                    *slot = id;
                }
                f(&ids)
            }

            fn static_type_info() -> Vec<TypeInfo> {
                let mut type_infos: Vec<TypeInfo> = vec![$(TypeInfo::of::<$name>()),*];
                type_infos.sort_by_key(|info| info.id());
                type_infos
            }

            #[allow(unused_variables)]
            unsafe fn get(archetype: &Archetype, index: usize) -> Result<Self, MissingComponent> where Self: Sized {
                Ok((
                    $(archetype.get_value::<$name>(index).unwrap(),)*
                ))
            }
        }
    }
}

macro_rules! count {
    () => { 0 };
    ($x: ident $(, $rest: ident)*) => { 1 + count!($($rest),*) };
}

smaller_tuples_too!(tuple_impl, O, N, M, L, K, J, I, H, G, F, E, D, C, B, A);
