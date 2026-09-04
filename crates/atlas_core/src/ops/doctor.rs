use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::capabilities::{Doctor, DoctorFinding, Severity};
use crate::registry::ComponentId;

/// The concatenated findings from `run_doctor`, in component order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub findings: Vec<DoctorFinding>,
}

/// A caller-implemented bound on one component's doctor probe, mirroring
/// `ReadinessDeadline`. `atlas_core` owns no timer.
#[async_trait]
pub trait DoctorDeadline: Send + Sync {
    /// Runs `probe`'s doctor check for `component` under the caller's
    /// bound. `None` means the bound elapsed before the probe completed.
    async fn bounded(
        &self,
        component: &ComponentId,
        probe: &dyn Doctor,
    ) -> Option<Vec<DoctorFinding>>;
}

/// Runs doctor sequentially across `components` (SHELL-OPS-4: "en
/// secuencia"), concatenating findings in call order. A component whose
/// probe exceeds its bound yields exactly one synthesized `Critical`
/// finding in its place, and aggregation continues to the next component.
/// Never accepts a `Readiness` value (INV-DOCTOR-NOT-READINESS).
pub async fn run_doctor(
    components: &[(ComponentId, &dyn Doctor)],
    deadline: &dyn DoctorDeadline,
) -> DoctorReport {
    let mut findings = Vec::new();

    for (component, probe) in components {
        match deadline.bounded(component, *probe).await {
            Some(mut component_findings) => findings.append(&mut component_findings),
            None => findings.push(DoctorFinding {
                component: component.clone(),
                severity: Severity::Critical,
                finding: "doctor check exceeded its deadline".to_string(),
                action: "investigate the slow diagnostic and retry".to_string(),
            }),
        }
    }

    DoctorReport { findings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::test_support::NeverElapses;

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    /// Elapses only for `elapsing`, running every other probe to completion
    /// — proves aggregation does not stop at the first timeout.
    struct ElapsesFor(ComponentId);

    #[async_trait]
    impl DoctorDeadline for ElapsesFor {
        async fn bounded(
            &self,
            component: &ComponentId,
            probe: &dyn Doctor,
        ) -> Option<Vec<DoctorFinding>> {
            if *component == self.0 {
                None
            } else {
                Some(probe.doctor().await)
            }
        }
    }

    fn finding(component_value: &str, finding: &str) -> DoctorFinding {
        DoctorFinding {
            component: component(component_value),
            severity: Severity::Info,
            finding: finding.to_string(),
            action: "no action required".to_string(),
        }
    }

    struct FixedDoctor(Vec<DoctorFinding>);

    #[async_trait]
    impl Doctor for FixedDoctor {
        async fn doctor(&self) -> Vec<DoctorFinding> {
            self.0.clone()
        }
    }

    #[test]
    fn run_doctor_concatenates_findings_in_component_order() {
        let custos = FixedDoctor(vec![finding("custos", "custos finding")]);
        let acta = FixedDoctor(vec![
            finding("acta", "acta finding one"),
            finding("acta", "acta finding two"),
        ]);
        let components: Vec<(ComponentId, &dyn Doctor)> =
            vec![(component("custos"), &custos), (component("acta"), &acta)];

        let report = crate::ops::test_support::block_on(run_doctor(&components, &NeverElapses));

        assert_eq!(
            report.findings,
            vec![
                finding("custos", "custos finding"),
                finding("acta", "acta finding one"),
                finding("acta", "acta finding two"),
            ]
        );
    }

    #[test]
    fn a_doctor_exceeding_its_deadline_yields_exactly_one_critical_finding_and_the_run_continues() {
        let slow = FixedDoctor(vec![finding("custos", "should not appear")]);
        let after = FixedDoctor(vec![finding("acta", "acta finding")]);
        let components: Vec<(ComponentId, &dyn Doctor)> =
            vec![(component("custos"), &slow), (component("acta"), &after)];

        let report = crate::ops::test_support::block_on(run_doctor(
            &components,
            &ElapsesFor(component("custos")),
        ));

        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.findings[0].component, component("custos"));
        assert_eq!(report.findings[0].severity, Severity::Critical);
        assert!(report.findings[0].finding.contains("exceeded its deadline"));
        assert_eq!(
            report.findings[1],
            finding("acta", "acta finding"),
            "the next component's doctor must still run after a timeout"
        );
    }

    #[test]
    fn run_doctor_never_accepts_a_readiness_value() {
        // Structural assertion mirroring readiness.rs's counterpart: this
        // call site only compiles with `&dyn Doctor` and `&dyn
        // DoctorDeadline` values. A stray `&dyn Readiness`/`ReadinessReport`
        // parameter on `run_doctor` would fail to compile here.
        let custos = FixedDoctor(vec![]);
        let components: Vec<(ComponentId, &dyn Doctor)> = vec![(component("custos"), &custos)];

        let _report = crate::ops::test_support::block_on(run_doctor(&components, &NeverElapses));
    }
}
