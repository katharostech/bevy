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
    DynamicQuery, DynamicSystemSettings, EntityBuilder, DYNAMIC_COMPONENT_INFO_REGISTRY,
};

// Set our component IDs. These should probably be hashes of a human-friendly but unique type
// identifier, depending on your scripting implementation needs. These identifiers should have data
// in the last 7 bits. See this comment for more info:
// https://users.rust-lang.org/t/requirements-for-hashes-in-hashbrown-hashmaps/50567/2?u=zicklag.
const POS_COMPONENT_ID: u64 = 242237625853274575;
const VEL_COMPONENT_ID: u64 = 6820197023594215835;

/// Create a system for spawning the scene
fn spawn_scene(world: &mut World, _resources: &mut Resources) {
    // Get the component info that we registered previously
    let pos_info = DYNAMIC_COMPONENT_INFO_REGISTRY
        .get_dynamic_component_info(POS_COMPONENT_ID)
        // This will panic if we have not previously registered the component info
        .unwrap();
    let vel_info = DYNAMIC_COMPONENT_INFO_REGISTRY
        .get_dynamic_component_info(VEL_COMPONENT_ID)
        .unwrap();

    // Here we will spawn our dynamically created components

    // For each entity we want to create, we must create a `RuntimeBundle` that contains all of that
    // entity's components. We're going to create a couple entities, each with two components, one
    // representing a Position and one representing a Velocity. Each of these will be made up of two
    // bytes for simplicity, one representing the x and y position/velocity.

    // We create our first entity
    let mut builder = EntityBuilder::new();
    // Then we add our "Position component"
    let entity1 = builder
        .add_dynamic(
            // We must specify the component information
            pos_info,
            // And provide the raw byte data data for the component
            vec![
                0, // X position byte
                0, // Y position byte
            ]
            // And cast the data to a pointer
            .as_slice(),
        )
        // Next we add our "Velocity component"
        .add_dynamic(
            vel_info,
            vec![
                0, // X position byte
                1, // Y position byte
            ]
            .as_slice(),
        )
        .build();

    // And let's create another entity
    let mut builder = EntityBuilder::new();
    let entity2 = builder
        .add_dynamic(
            pos_info,
            vec![
                0, // X position byte
                0, // Y position byte
            ]
            .as_slice(),
        )
        .add_dynamic(
            vel_info,
            vec![
                2, // X position byte
                0, // Y position byte
            ]
            .as_slice(),
        )
        .build();

    // Now we can spawn our entities
    world.spawn(entity1);
    world.spawn(entity2);
}

fn main() {
    // Register information related to our component types and get their info
    let pos_info = DYNAMIC_COMPONENT_INFO_REGISTRY
        .register_dynamic_component(
            POS_COMPONENT_ID,
            Layout::from_size_align(2, 1).unwrap(),
            None,
        )
        // This will panic if the component has already been registered
        .unwrap();
    let vel_info = DYNAMIC_COMPONENT_INFO_REGISTRY
        .register_dynamic_component(
            VEL_COMPONENT_ID,
            Layout::from_size_align(2, 1).unwrap(),
            None,
        )
        .unwrap();

    // A Dynamic component query which can be constructed at runtime to represent which components
    // we want a dynamic system to access.
    //
    // Notice that the sizes and IDs of the components can be specified at runtime and allow for
    // storage of any data as an array of bytes.
    let mut query = DynamicQuery::default();

    // Query the entity as well
    query.entity = true;

    // We need to query our "velocity" component by specifying its ID and size.
    query.immutable.push(vel_info);

    // We need to query our "position" component by specifying its ID and size
    query.mutable.push(pos_info);

    // Create a dynamic system with the query we constructed
    let pos_vel_system =
        DynamicSystem::new("pos_vel_system".into(), () /* system local state */).settings(
            DynamicSystemSettings {
                queries: vec![query],
                workload: |_state, _resources, mut queries| {
                    // Print a spacer
                    println!("-----");

                    // Iterate over the query
                    for mut components in queries[0].iter_mut() {
                        // Would panic if we had not set `query.entity = true`
                        let entity = components.entity.unwrap();
                        let pos_bytes = &mut components.mutable[0];
                        let vel_bytes = &components.immutable[0];

                        // Add the X velocity to the X position
                        pos_bytes[0] += vel_bytes[0];
                        // And the same with the Y
                        pos_bytes[1] += vel_bytes[1];

                        // Print out the position and velocity
                        println!(
                            "Entity: {:?}\tPosition: {:?}\tVelocity: {:?}",
                            entity, pos_bytes, vel_bytes
                        );
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
