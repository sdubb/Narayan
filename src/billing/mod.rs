//! Billing module — step-based billing with pluggable payment providers.
//!
//! Plans (all get all connectors + compliance):
//!   Free:       $0   /mo  — 1,000  steps/mo, 3  concurrent agents
//!   Go:         $15  /mo  — 20,000 steps/mo, 20 concurrent agents
//!   Pro:        $79  /mo  — 150,000 steps/mo, 200 concurrent agents
//!   Enterprise: custom    — unlimited
//!
//! Credit top-ups: $8 per 5,000 extra steps (any paid plan).
//!
//! Adding a new provider (e.g. Razorpay):
//!   1. Create src/billing/razorpay.rs implementing BillingProvider
//!   2. Add .register(Arc::new(RazorpayProvider::from_env()?)) in main.rs
//!   3. The webhook route /billing/webhooks/razorpay works automatically

pub mod provider;
pub mod store;
pub mod paypal;
pub mod stripe;
pub mod routes;

pub use provider::{BillingEvent, BillingPlan, BillingProvider, CheckoutSession, ProviderSubscription};
pub use store::BillingStore;
