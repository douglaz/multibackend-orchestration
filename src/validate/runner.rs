use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::validate::harness::RalphHarness;
use crate::Result;

#[derive(Debug, Clone)]
pub struct ConformanceTest {
    pub name: &'static str,
    pub func: fn(&RalphHarness) -> TestResult,
}

#[derive(Debug, Clone)]
pub enum TestResult {
    Pass,
    Fail(String),
    Skip(String),
}

#[derive(Debug)]
pub struct TestRunner {
    tests: Vec<ConformanceTest>,
    ralph_bin: PathBuf,
    filter: Option<String>,
    verbose: bool,
}

impl TestRunner {
    pub fn new(
        tests: Vec<ConformanceTest>,
        ralph_bin: PathBuf,
        filter: Option<String>,
        verbose: bool,
    ) -> Self {
        Self {
            tests,
            ralph_bin,
            filter,
            verbose,
        }
    }

    pub fn run(&self, list_only: bool) -> Result<bool> {
        let filtered = self.filtered_tests();

        println!("running {} tests", filtered.len());

        if list_only {
            for test in filtered {
                println!("test {}", test.name);
            }
            println!();
            println!("test result: ok. 0 passed; 0 failed; 0 skipped");
            return Ok(true);
        }

        let mut passed = 0usize;
        let mut skipped: Vec<(&'static str, String)> = Vec::new();
        let mut failures: Vec<(&'static str, String)> = Vec::new();
        let mut durations: Vec<(&'static str, Duration)> = Vec::new();

        for test in filtered {
            let start = Instant::now();
            let harness = RalphHarness::new(&self.ralph_bin)?;
            let result = (test.func)(&harness);
            let elapsed = start.elapsed();
            durations.push((test.name, elapsed));
            match result {
                TestResult::Pass => {
                    passed += 1;
                    println!("test {} ... ok ({})", test.name, format_duration(elapsed));
                }
                TestResult::Skip(reason) => {
                    println!("test {} ... skipped ({})", test.name, format_duration(elapsed));
                    if self.verbose {
                        for line in reason.lines() {
                            println!("  {line}");
                        }
                    }
                    skipped.push((test.name, reason));
                }
                TestResult::Fail(message) => {
                    println!("test {} ... FAILED ({})", test.name, format_duration(elapsed));
                    if self.verbose {
                        for line in message.lines() {
                            println!("  {line}");
                        }
                    }
                    failures.push((test.name, message));
                }
            }
        }

        if !failures.is_empty() {
            println!();
            println!("failures:");
            println!();
            for (name, message) in &failures {
                println!("--- {name} ---");
                for line in message.lines() {
                    println!("  {line}");
                }
                println!();
            }
        }

        let failed = failures.len();
        let skipped_count = skipped.len();

        if failed == 0 {
            let total = durations.iter().map(|(_, duration)| duration).sum::<Duration>();
            println!("test result: ok. {passed} passed; 0 failed; {skipped_count} skipped");
            println!("test result: total {}", format_duration(total));
            return Ok(true);
        }

        let total = durations.iter().map(|(_, duration)| duration).sum::<Duration>();
        println!(
            "test result: FAILED. {passed} passed; {failed} failed; {skipped_count} skipped"
        );
        println!("test result: total {}", format_duration(total));
        Ok(false)
    }

    fn filtered_tests(&self) -> Vec<&ConformanceTest> {
        self.tests
            .iter()
            .filter(|test| {
                if let Some(filter) = &self.filter {
                    test.name.contains(filter)
                } else {
                    true
                }
            })
            .collect()
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let millis = duration.subsec_millis();
    format!("{seconds}.{millis:03}s")
}
