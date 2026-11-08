//! A generic-only library. Source facts must survive without a reachable provider item.
#![no_std]

pub async fn relay<T>(value: T) -> T {
    core::future::ready(()).await;
    value
}

pub async fn repeated<T: Copy>(value: T, count: u32) -> T {
    for _ in 0..count {
        core::future::ready(()).await;
    }
    value
}

pub async fn immediate<T>(value: T) -> T {
    value
}
