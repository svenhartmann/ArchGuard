//! `archguard-flow-backend` — L2: the Spring and ASP.NET UI-flow driver
//! (ADR 0021): one pass over `.cs`, `.java` and `.kt` files with a shared
//! index and output, plus the WebFlux functional routes and the Gradle/Maven
//! module facts of the applications.

mod ui_flow_apps;
pub mod ui_flow_backend;
pub mod ui_flow_webflux;
