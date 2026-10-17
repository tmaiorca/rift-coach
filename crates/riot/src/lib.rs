//! Riot Web API client. Every request goes through one shared [`RateLimiter`],
//! which enforces app- and method-level limits per routing value and honors
//! `Retry-After` on 429s.

pub mod client;
pub mod dto;
pub mod error;
pub mod limiter;
pub mod routing;

pub use client::{Endpoint, RiotClient};
pub use error::RiotError;
pub use limiter::{LimitWindow, RateLimiter};
pub use routing::{Platform, Region, RiotId};
