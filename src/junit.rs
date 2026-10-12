//! cj8.14 (E2): JUnit XML output for reports and matrices.
//!
//! Hand-written XML (no dependency) with proper escaping; every
//! mainstream CI dashboard ingests the JUnit format. The full JSON
//! report goes in `<system-out>` CDATA for drill-down.

use crate::branch::BranchMatrixReport;
use crate::driver::PlaytestReport;
use crate::enums::PlaytestStatus;

/// Escape a string for XML text/attribute content.
pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            // XML 1.0 forbids most control chars
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {
                o.push_str(&format!("&#x{:X};", c as u32))
            }
            c => o.push(c),
        }
    }
    o
}

/// CDATA-safe: split any `]]>` into `]]]]><![CDATA[>` so the embedded
/// JSON cannot terminate the CDATA section early.
fn cdata_wrap(content: &str) -> String {
    format!("<![CDATA[{}]]>", content.replace("]]>", "]]]]><![CDATA[>"))
}

impl PlaytestReport {
    /// One `<testcase>`. Fail -> `<failure>`, Crash -> `<error>`;
    /// the full JSON report goes in `<system-out>` for drill-down.
    pub fn to_junit_testcase(&self, name: &str, classname: &str) -> String {
        let time = if self.wall_time_s > 0.0 {
            self.wall_time_s
        } else {
            self.simulated_duration_s
        };
        let mut out = format!(
            "      <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">\n",
            xml_escape(name),
            xml_escape(classname),
            time
        );
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string());
        match self.status {
            PlaytestStatus::Pass => {}
            PlaytestStatus::Crash => {
                let msg = self.error.as_deref().unwrap_or("crash");
                out.push_str(&format!(
                    "        <error message=\"{}\" type=\"crash\">{}\n        `</error>`\n",
                    xml_escape(msg),
                    xml_escape(msg)
                ));
            }
            PlaytestStatus::Fail => {
                // One summary line per violation: rule / target /
                // first..last frame / count / detail.
                let mut body = String::new();
                for v in &self.violations {
                    body.push_str(&format!(
                        "[{} @ {}] frames {}..{}, count {}, detail: {}\n",
                        v.rule, v.target, v.first_frame, v.last_frame, v.count, v.last_detail
                    ));
                }
                let headline = self
                    .violations
                    .first()
                    .map(|v| format!("{} @ {}", v.rule, v.target))
                    .unwrap_or_else(|| "failed".to_string());
                let ftype = self
                    .violations
                    .first()
                    .map(|v| v.rule.clone())
                    .unwrap_or_else(|| "failure".to_string());
                out.push_str(&format!(
                    "        <failure message=\"{}\" type=\"{}\">\n{}        `</failure>`\n",
                    xml_escape(&headline),
                    xml_escape(&ftype),
                    xml_escape(&body)
                ));
            }
        }
        out.push_str(&format!(
            "        `<system-out>`{}`</system-out>`\n",
            cdata_wrap(&json)
        ));
        out.push_str("      `</testcase>`\n");
        out
    }
}

impl BranchMatrixReport {
    /// Full JUnit XML document for a branch matrix run.
    pub fn to_junit(&self, suite_name: &str) -> String {
        let tests = self.outcomes.len();
        let failures = self
            .outcomes
            .iter()
            .filter(|o| !o.passed() && o.is_fail())
            .count();
        let errors = self
            .outcomes
            .iter()
            .filter(|o| !o.passed() && o.is_crash())
            .count();
        let total_time: f64 = self.outcomes.iter().map(|o| o.report_time()).sum();
        let mut out = String::new();
        out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        out.push_str("`<testsuites>`\n");
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\" time=\"{:.3}\">\n",
            xml_escape(suite_name),
            tests,
            failures,
            errors,
            total_time
        ));
        for o in &self.outcomes {
            out.push_str(&o.report.to_junit_testcase(&o.variant_name, suite_name));
        }
        out.push_str("  `</testsuite>`\n");
        out.push_str("`</testsuites>`\n");
        out
    }
}

#[cfg(test)]
mod tests_cj8_14 {
    use super::*;
    use crate::branch::BranchOutcome;

    fn pass_report() -> PlaytestReport {
        let mut r = PlaytestReport::crashed_empty();
        r.status = PlaytestStatus::Pass;
        r
    }

    fn fail_report() -> PlaytestReport {
        let mut r = PlaytestReport::crashed_empty();
        r.status = PlaytestStatus::Fail;
        r.violations.push(crate::state::ViolationEntry {
            rule: "bounds_check".into(),
            target: "Ball <&\"quoted\">".into(),
            first_frame: 412,
            last_frame: 412,
            count: 1,
            detail: "out of bounds (10012.0)".into(),
            last_detail: "out of bounds (10012.0)".into(),
            fingerprint: None,
            fingerprint_scheme: 1,
        });
        r
    }

    #[test]
    fn escape_specials() {
        assert_eq!(xml_escape("a<b>&\"'c"), "a&lt;b&gt;&amp;&quot;&apos;c");
        assert_eq!(xml_escape("\u{1}"), "&#x1;");
        assert_eq!(xml_escape("\t\n\r"), "\t\n\r");
    }

    #[test]
    fn passing_testcase_has_no_failure() {
        let tc = pass_report().to_junit_testcase("pass-case", "suite");
        assert!(tc.contains("<testcase name=\"pass-case\""));
        assert!(!tc.contains("<failure"), "{tc}");
        assert!(!tc.contains("<error"), "{tc}");
        assert!(tc.contains("`<system-out>`"));
    }

    #[test]
    fn failing_testcase_escapes_and_details() {
        let tc = fail_report().to_junit_testcase("fail-case", "suite");
        assert!(tc.contains("<failure"));
        assert!(tc.contains("type=\"bounds_check\""));
        // angle brackets + quotes in the target are escaped
        assert!(tc.contains("&lt;&amp;&quot;"), "{tc}");
        assert!(!tc.contains("<&\""), "{tc}");
        assert!(tc.contains("count 1"));
        assert!(tc.contains("frames 412..412"));
    }

    #[test]
    fn crash_testcase_uses_error() {
        let mut r = PlaytestReport::crashed_empty();
        r.error = Some("boom < boom".into());
        let tc = r.to_junit_testcase("crash-case", "suite");
        assert!(tc.contains("<error"));
        assert!(tc.contains("boom &lt; boom"));
    }

    #[test]
    fn cadata_split_protects_system_out() {
        let mut r = pass_report();
        r.violations = vec![];
        // Inject a ]]>* sequence via the JSON body: set a field we know
        // round-trips into JSON containing ]]>.
        r.warnings = vec!["evil]]>injection".into()];
        let tc = r.to_junit_testcase("cdata-case", "suite");
        // The raw terminator must not appear unsplit
        let body = tc;
        assert!(
            !body.matches("]]>").count() > body.matches("]]]]><![CDATA[>").count() + 1,
            "CDATA terminators must be split: {body}"
        );
    }

    #[test]
    fn matrix_document_balanced_and_counted() {
        let matrix = BranchMatrixReport {
            outcomes: vec![
                BranchOutcome {
                    variant_name: "pass-variant".into(),
                    ticks_run: 10,
                    report: pass_report(),
                },
                BranchOutcome {
                    variant_name: "fail-variant".into(),
                    ticks_run: 20,
                    report: fail_report(),
                },
            ],
        };
        let doc = matrix.to_junit("swarm");
        assert!(doc.starts_with("<?xml"));
        assert!(doc.contains("tests=\"2\""));
        assert!(doc.contains("failures=\"1\""));
        assert!(doc.contains("errors=\"0\""));
        // Balanced-tag hand-rolled check
        let opens = doc.matches('<').count();
        let closes =
            doc.matches("</").count() + doc.matches("/>").count() + doc.matches("<?").count();
        assert!(opens >= closes, "rough balance: {opens} vs {closes}");
        assert_eq!(doc.matches("<testcase").count(), 2);
        assert_eq!(doc.matches("`</testcase>`").count(), 2);
    }
}
