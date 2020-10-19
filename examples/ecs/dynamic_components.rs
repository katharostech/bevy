//! This example demonstrates how to create systems and queryies for those systems at runtime
//!
//! The primary use-case for doing so would be allow for integrations with scripting languages,
//! where you do no have the information about what systems exist or what queries they will make.
//!
//! In this example the components are `repr(C)` Rust structs that are spawned from Rust code. To
//! see how to also spawn entities with runtime created Components check out the

use std::{alloc::Layout, time::Duration};

use bevy::prelude::*;
use bevy_app::ScheduleRunnerPlugin;
use bevy_ecs::{
    ComponentId, DynamicComponentInfo, DynamicComponentQuery, DynamicSystemSettings, RuntimeBundle,
    TypeInfo,
};

/// Create a system for spawning the scene
fn spawn_scene(world: &mut World, _resources: &mut Resources) {
    // Here we will spawn our dynamically created components

    // For each entity we want to create, we must create a `RuntimeBundle` that contains all of that
    // entity's components. We're going to create a couple entities, each with two components, one
    // representing a Position and one representing a Velocity. Each of these will be made up of two
    // bytes for simplicity, one representing the x and y position/velocity.

    // We create our first component bundle
    let components1 = RuntimeBundle {
        // we must define our components' type information
        components: vec![
            // First we define our "Position" component
            TypeInfo {
                // We must provide a unique id for the compoonent
                id: ComponentId::ExternalId(0),
                // And we must specify the size and alignment of the component
                layout: Layout::from_size_align(2 /* size */, 1 /* alignment */).unwrap(),
                // And we must specify a drop function for our component
                drop: |_| (),
            },
            // Next we define our "Velocity" component
            TypeInfo {
                // We must specify a different ID for the velocity component
                id: ComponentId::ExternalId(1),
                // We specify the layout which happens to be the same as "Position"
                layout: Layout::from_size_align(2, 1).unwrap(),
                // And the drop function
                drop: |_| (),
            },
        ],

        // Data must be a Vector of Vectors of bytes and must contain the raw byte data for
        // each of the components we want to add
        data: vec![
            // This will be the raw byte data for our position component
            vec![
                0, // X position byte
                0, // Y position byte
            ],
            // This will be the raw byte data for our velocity component
            vec![
                1, // X velocity byte
                0, // Y velocity byte
            ],
        ],
    };

    // Now we create another bundle for our next entity
    let components2 = RuntimeBundle {
        components: vec![
            TypeInfo {
                id: ComponentId::ExternalId(0),
                layout: Layout::from_size_align(2 /* size */, 1 /* alignment */).unwrap(),
                drop: |_| (),
            },
            TypeInfo {
                id: ComponentId::ExternalId(1),
                layout: Layout::from_size_align(2, 1).unwrap(),
                drop: |_| (),
            },
        ],
        data: vec![vec![0, 0], vec![0, 2]],
    };

    // Now we can spawn our entities
    world.spawn(components1);
    world.spawn(components2);
}

fn main() {
    // A Dynamic component query which can be constructed at runtime to represent which components
    // we want a dynamic system to access.
    //
    // Notice that the sizes and IDs of the components can be specified at runtime and allow for
    // storage of any data as an array of bytes.
    let mut query = DynamicComponentQuery::default();

    // We need to query our "velocity" component by specifying its ID and size.
    query.immutable[0] = Some(DynamicComponentInfo {
        id: ComponentId::ExternalId(1),
        size: 2,
    });

    // We need to query our "position" component by specifying its ID and size
    query.mutable[0] = Some(DynamicComponentInfo {
        id: ComponentId::ExternalId(0),
        size: 2,
    });

    // Create a dynamic system with the query we constructed
    let pos_vel_system =
        DynamicSystem::new("pos_vel_system".into(), () /* system local state */).settings(
            DynamicSystemSettings {
                queries: vec![query],
                workload: |_state, _resources, queries| {
                    // Print a spacer
                    println!("-----");

                    // Iterate over the query
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

                        // Add the X velocity to the X position
                        pos_bytes[0] += vel_bytes[0];
                        // And the same with the Y
                        pos_bytes[1] += vel_bytes[1];

                        // Print out the position and velocity
                        println!("Position: {:?}\tVelocity: {:?}", pos_bytes, vel_bytes);
                    }
                },
                ..Default::default()
            },
        );

    App::build()
        .add_plugin(ScheduleRunnerPlugin::run_loop(Duration::from_secs(1)))
        .add_startup_system(spawn_scene.thread_local_system())
        .add_system(Box::new(pos_vel_system))
        .run();
}
