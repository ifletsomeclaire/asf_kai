use bevy_ecs::prelude::*;
use std::thread;
use std::time::{Duration, Instant};
use log;

#[derive(Resource, Default)]
struct ResourceA(u32);

#[derive(Resource, Default)]
struct ResourceB(u32);

fn system_a(mut res_a: ResMut<ResourceA>) {
    thread::sleep(Duration::from_millis(100));
    res_a.0 += 1;
}

fn system_b(mut res_b: ResMut<ResourceB>) {
    thread::sleep(Duration::from_millis(100));
    res_b.0 += 1;
}

#[test]
fn test_parallel_execution() {
    // Initialize logger for tests
    let _ = env_logger::try_init();
    
    let mut world = World::default();
    world.init_resource::<ResourceA>();
    world.init_resource::<ResourceB>();

    let mut schedule = Schedule::default();
    schedule.add_systems((system_a, system_b));

    let start_time = Instant::now();
    schedule.run(&mut world);
    let elapsed = start_time.elapsed();

    log::info!("Parallel execution time: {:?}", elapsed);

    // If the systems ran in parallel, the total time should be slightly over 100ms.
    // If they ran sequentially, it would be over 200ms.
    // We allow for some overhead.
    assert!(elapsed < Duration::from_millis(200));
    assert!(elapsed > Duration::from_millis(100));

    let res_a = world.resource::<ResourceA>();
    let res_b = world.resource::<ResourceB>();

    assert_eq!(res_a.0, 1);
    assert_eq!(res_b.0, 1);
}

fn sequential_system_a(mut res_a: ResMut<ResourceA>) {
    thread::sleep(Duration::from_millis(100));
    res_a.0 += 1;
}

fn sequential_system_b(mut res_a: ResMut<ResourceA>) {
    thread::sleep(Duration::from_millis(100));
    res_a.0 += 1;
}

#[test]
fn test_sequential_execution() {
    // Initialize logger for tests
    let _ = env_logger::try_init();
    
    let mut world = World::default();
    world.init_resource::<ResourceA>();

    let mut schedule = Schedule::default();
    schedule.add_systems(sequential_system_a);
    schedule.add_systems(sequential_system_b);

    let start_time = Instant::now();
    schedule.run(&mut world);
    let elapsed = start_time.elapsed();

    log::info!("Sequential execution time: {:?}", elapsed);

    // If the systems ran sequentially, the total time should be over 200ms.
    assert!(elapsed > Duration::from_millis(200));

    let res_a = world.resource::<ResourceA>();
    assert_eq!(res_a.0, 2);
}
