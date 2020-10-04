//! This example demonstrates how to create systems and queryies for those systems at runtime
//!
//! The primary use-case for doing so would be allow for integrations with scripting languages,
//! where you do no have the information about what systems exist or what queries they will make.
//!
//! In this example the components are `repr(C)` Rust structs that are spawned from Rust code. To
//! see how to also spawn entities with runtime created Components check out the

use std::time::Duration;

use bevy::prelude::*;
use bevy_app::ScheduleRunnerPlugin;
use bevy_ecs::{ComponentId, DynamicComponentInfo, DynamicComponentQuery, DynamicSystemSettings};

// Define our components

#[derive(Debug, Clone, Copy)]
struct Pos {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, Copy)]
struct Vel {
    x: f32,
    y: f32,
}

/// Create a system for spawning the scene
fn spawn_scene(world: &mut World, _resources: &mut Resources) {
    #[rustfmt::skip]
    world.spawn_batch(vec![
        (
            Pos {
                x: 0.0,
                y: 0.0
            },
            Vel {
                x: 0.0,
                y: -1.0,
            }
        ),
        (
            Pos {
                x: 0.0,
                y: 0.0
            },
            Vel {
                x: 0.0,
                y: 1.0,
            }
        ),
        (
            Pos {
                x: 1.0,
                y: 1.0
            },
            Vel {
                x: -0.5,
                y: 0.5,
            }
        ),
    ]);
}

/// Create a system for printing the status of our entities
fn info(mut query: Query<(&Pos, &Vel)>) {
    println!("---");
    for (pos, vel) in &mut query.iter() {
        println!("{:?}\t\t{:?}", pos, vel);
    }
}

fn main() {
    // A Dynamic component query which can be constructed at runtime to represent which components
    // we want a dynamic system to access.
    //
    // Notice that the sizes and IDs of the components can be specified at runtime and allow for
    // storage of any data as an array of bytes.
    let mut query = DynamicComponentQuery::default();

    // Add an immutable query for `Vel`
    query.immutable[0] = Some(DynamicComponentInfo {
        id: ComponentId::RustTypeId(std::any::TypeId::of::<Vel>()),
        size: std::mem::size_of::<Vel>(),
    });

    // Add a mutable query for `Pos`
    query.mutable[0] = Some(DynamicComponentInfo {
        id: ComponentId::RustTypeId(std::any::TypeId::of::<Pos>()),
        size: std::mem::size_of::<Pos>(),
    });

    // Create a dynamic system
    let pos_vel_system = DynamicSystem::new(
        "pos_vel_system".into(),
        (), /* system local state, can be any type */
    )
    .settings(
        // Specify the settings for our dynamic system
        DynamicSystemSettings {
            // Specify all of our queries
            queries: vec![
                // In this case we only have one query, but there could be multiple
                query,
            ],
            workload: |_state, _resources, queries| {
                // Grat the first ( and only ) query out of the passed in queries and iterate
                // over it.
                for mut components in &mut queries[0].iter() {
                    // `components` will be an array with indexes corresponding to the indexes of our
                    // DynamicComponentAccess information that we constructed for our query when creating
                    // the system.
                    //
                    // Each item in the array is an optional mutable reference to a byte slice representing
                    // the component data: Option<&mut [u8]>.

                    // Here we take the mutable reference to the bytes of our position and velocity
                    // components
                    let pos_bytes = components.mutable[0].take().unwrap();
                    let vel_bytes = components.immutable[0].take().unwrap();

                    unsafe fn from_slice_mut<T>(s: &mut [u8]) -> &mut T {
                        debug_assert_eq!(std::mem::size_of::<T>(), s.len());
                        &mut *(s.as_mut_ptr() as *mut T)
                    }

                    unsafe fn from_slice<T>(s: &[u8]) -> &T {
                        debug_assert_eq!(std::mem::size_of::<T>(), s.len());
                        &*(s.as_ptr() as *mut T)
                    }

                    // Instead of interacting with the raw bytes of our components, we first cast them to
                    // their Rust structs
                    let mut pos: &mut Pos = unsafe { from_slice_mut(pos_bytes) };
                    let vel: &Vel = unsafe { from_slice(vel_bytes) };

                    // Now we can operate on our components
                    pos.x += vel.x;
                    pos.y += vel.y;
                }
            },
            ..Default::default()
        },
    );

    App::build()
        .add_plugin(ScheduleRunnerPlugin::run_loop(Duration::from_secs(1)))
        .add_startup_system(spawn_scene.thread_local_system())
        .add_system(Box::new(pos_vel_system))
        .add_system(info.system())
        .run();
}
