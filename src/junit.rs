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
            // le3.6 (RF2): XML 1.0 forbids C0 control chars (except
            // \t\n\r) AND character references to them, plus
            // U+FFFE/U+FFFF. Replace with U+FFFD (replacement char)
            // so documents stay well-formed regardless of input.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => o.push('\u{FFFD}'),
            '\u{FFFE}' | '\u{FFFF}' => o.push('\u{FFFD}'),
            c => o.push(c),
        }
    }
    o
}

/// CDATA-safe: split any `]]>` into `]]]]><![CDATA[>` so the embedded
/// JSON cannot terminate the CDATA section early.
fn cdata_wrap(content: &str) -> String {
    // le3.6 (RF2): control chars are illegal inside CDATA too —
    // filter them like xml_escape (keeping \t\n\r), then split any
    // `]]>` so the payload can't terminate the section early.
    let filtered: String = content
        .chars()
        .map(|c| {
            if ((c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r'))
                || matches!(c, '\u{FFFE}' | '\u{FFFF}')
            {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect();
    format!("<![CDATA[{}]]>", filtered.replace("]]>", "]]]]><![CDATA[>"))
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
                    "        <error message=\"{}\" type=\"crash\">{}\n        </error>\n",
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
                    "        <failure message=\"{}\" type=\"{}\">\n{}        </failure>\n",
                    xml_escape(&headline),
                    xml_escape(&ftype),
                    xml_escape(&body)
                ));
            }
        }
        out.push_str(&format!(
            "        <system-out>{}</system-out>\n",
            cdata_wrap(&json)
        ));
        out.push_str("      </testcase>\n");
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
        out.push_str("<testsuites>\n");
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
        out.push_str("  </testsuite>\n");
        out.push_str("</testsuites>\n");
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

    /// le3.6 (RF2): parse any fragment/document with a real XML
    /// parser. Panics on malformed XML.
    fn assert_well_formed(xml: &str) {
        let mut reader = quick_xml::Reader::from_str(xml);
        reader.config_mut().check_end_names = true;
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("malformed XML ({e}): {xml}"),
            }
        }
    }

    #[test]
    fn escape_specials() {
        assert_eq!(xml_escape("a<b>&\"'c"), "a&lt;b&gt;&amp;&quot;&apos;c");
        // le3.6 (RF2): C0 controls and U+FFFE/U+FFFF are REPLACED
        // (character references to them are illegal in XML 1.0).
        assert_eq!(xml_escape("\u{1}"), "\u{FFFD}");
        assert_eq!(xml_escape("\u{0}\u{7}\u{1b}"), "\u{FFFD}\u{FFFD}\u{FFFD}");
        assert_eq!(xml_escape("\u{FFFE}\u{FFFF}"), "\u{FFFD}\u{FFFD}");
        // allowed whitespace passes through
        assert_eq!(xml_escape("\t\n\r"), "\t\n\r");
    }

    #[test]
    fn passing_testcase_parses_and_has_no_failure() {
        let tc = pass_report().to_junit_testcase("pass-case", "suite");
        // le3.6 (RF2): REAL parse, no backtick string assertions.
        assert_well_formed(&format!("<root>{tc}</root>"));
        assert!(tc.contains("<testcase name=\"pass-case\""));
        assert!(!tc.contains("<failure"), "{tc}");
        assert!(!tc.contains("<error"), "{tc}");
        assert!(tc.contains("<system-out>"));
        assert!(!tc.contains('`'), "no literal backticks: {tc}");
    }

    #[test]
    fn failing_testcase_escapes_and_details() {
        let tc = fail_report().to_junit_testcase("fail-case", "suite");
        assert_well_formed(&format!("<root>{tc}</root>"));
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
        assert_well_formed(&format!("<root>{tc}</root>"));
        assert!(tc.contains("<error"));
        assert!(tc.contains("boom &lt; boom"));
    }

    /// le3.6 (RF2): violation detail containing ANSI escapes / NUL /
    /// U+FFFE must yield WELL-FORMED XML (previously emitted illegal
    /// &#x1;-style refs).
    #[test]
    fn control_chars_yield_well_formed_xml() {
        let mut r = fail_report();
        r.violations[0].detail = "\u{1b}[31m red \u{0} bell \u{7}".into();
        r.violations[0].last_detail = "\u{1b}[31m red \u{0} bell \u{7}".into();
        r.error = Some("\u{FFFE} bad \u{FFFF}".into());
        let tc = r.to_junit_testcase("ctrl-case", "suite");
        assert_well_formed(&format!("<root>{tc}</root>"));
        assert!(
            !tc.contains("&#x"),
            "no char refs to forbidden codepoints: {tc}"
        );
    }

    /// le3.6 (RF2): CDATA round-trip — parse the testcase, extract the
    /// system-out text, and verify it equals the report JSON (with
    /// the ]]> split undone). Replaces the vacuous bitwise-NOT
    /// assertion.
    #[test]
    fn cdata_split_protects_system_out() {
        let mut r = pass_report();
        r.warnings = vec!["evil]]>injection".into()];
        let tc = r.to_junit_testcase("cdata-case", "suite");
        assert_well_formed(&format!("<root>{tc}</root>"));
        // Round-trip: strip CDATA wrappers, undo the split, compare
        // with the serialized JSON.
        let inner_start = tc.find("<system-out>").unwrap() + "<system-out>".len();
        let inner_end = tc.rfind("</system-out>").unwrap();
        let inner = &tc[inner_start..inner_end];
        assert!(
            inner.starts_with("<![CDATA["),
            "CDATA wrapper missing: {inner}"
        );
        let content = inner
            .strip_prefix("<![CDATA[")
            .and_then(|s| s.strip_suffix("]]>"))
            .expect("CDATA terminator");
        let rejoined = content.replace("]]]]><![CDATA[>", "]]>");
        let expected = serde_json::to_string_pretty(&r).unwrap();
        assert_eq!(rejoined, expected, "CDATA payload must round-trip");
        // The CONTENT after rejoining DOES contain ]]> (from the JSON).
        // What matters: the RAW XML had the split, so the CDATA section
        // wasn't terminated early. Verify by checking the original XML
        // contains the split pattern (]]]]><![CDATA[>) but not an
        // unescaped ]]> inside the CDATA wrapper.
        assert!(inner.contains("]]]]><![CDATA[>"), "split missing: {inner}");
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
        // le3.6 (RF2): REAL parse of the full document (replaces the
        // trivially-true opens>=closes heuristic).
        assert_well_formed(&doc);
        assert!(doc.starts_with("<?xml"));
        assert!(doc.contains("tests=\"2\""));
        assert!(doc.contains("failures=\"1\""));
        assert!(doc.contains("errors=\"0\""));
        assert_eq!(doc.matches("<testcase").count(), 2);
        assert_eq!(doc.matches("</testcase>").count(), 2);
        assert!(!doc.contains('`'), "no literal backticks in document");
    }
}
