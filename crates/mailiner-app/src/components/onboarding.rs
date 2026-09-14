//! First-run onboarding: multi-step account setup (connect-before-persist).

use dioxus::prelude::*;

use crate::components::account_setup::AccountSetupWizard;
use crate::setup_wizard::SetupMode;

/// First-run account wizard. Connect-before-persist via `CommitNewAccount`.
#[component]
pub fn OnboardingForm() -> Element {
    rsx! {
        AccountSetupWizard { mode: SetupMode::FirstRun }
    }
}
