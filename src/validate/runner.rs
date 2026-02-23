use std::path::PathBuf;
use std::sync::Mutex;
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
    jobs: usize,
}

impl TestRunner {
    pub fn new(
        tests: Vec<ConformanceTest>,
        ralph_bin: PathBuf,
        filter: Option<String>,
        verbose: bool,
        jobs: usize,
    ) -> Self {
        Self {
            tests,
            ralph_bin,
            filter,
            verbose,
            jobs: jobs.max(1),
        }
    }

    pub fn run(&self, list_only: bool) -> Result<bool> {
        let filtered = self.filtered_tests();

        println!("running {} tests (jobs: {})", filtered.len(), self.jobs);

        if list_only {
            for test in filtered {
                println!("test {}", test.name);
            }
            println!();
            println!("test result: ok. 0 passed; 0 failed; 0 skipped");
            return Ok(true);
        }

        if self.jobs == 1 {
            self.run_sequential(&filtered)
        } else {
            self.run_parallel(&filtered)
        }
    }

    fn run_sequential(&self, filtered: &[&ConformanceTest]) -> Result<bool> {
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
                    println!(
                        "test {} ... skipped ({})",
                        test.name,
                        format_duration(elapsed)
                    );
                    if self.verbose {
                        for line in reason.lines() {
                            println!("  {line}");
                        }
                    }
                    skipped.push((test.name, reason));
                }
                TestResult::Fail(message) => {
                    println!(
                        "test {} ... FAILED ({})",
                        test.name,
                        format_duration(elapsed)
                    );
                    if self.verbose {
                        for line in message.lines() {
                            println!("  {line}");
                        }
                    }
                    failures.push((test.name, message));
                }
            }
        }

        self.print_summary(&failures, passed, skipped.len(), &durations)
    }

    fn run_parallel(&self, filtered: &[&ConformanceTest]) -> Result<bool> {
        let work = Mutex::new(filtered.iter().enumerate());
        let results: Mutex<Vec<(usize, &'static str, TestResult, Duration)>> =
            Mutex::new(Vec::new());
        let harness_error: Mutex<Option<crate::error::RalphError>> = Mutex::new(None);

        std::thread::scope(|s| {
            for _ in 0..self.jobs {
                s.spawn(|| {
                    loop {
                        let (idx, test) = {
                            let mut q = work.lock().unwrap();
                            match q.next() {
                                Some((i, t)) => (i, *t),
                                None => break,
                            }
                        };
                        let start = Instant::now();
                        let harness = match RalphHarness::new(&self.ralph_bin) {
                            Ok(h) => h,
                            Err(e) => {
                                *harness_error.lock().unwrap() = Some(e);
                                break;
                            }
                        };
                        let result = (test.func)(&harness);
                        let elapsed = start.elapsed();
                        results.lock().unwrap().push((idx, test.name, result, elapsed));
                    }
                });
            }
        });

        if let Some(e) = harness_error.into_inner().unwrap() {
            return Err(e);
        }

        let mut results = results.into_inner().unwrap();
        results.sort_by_key(|(idx, _, _, _)| *idx);

        let mut passed = 0usize;
        let mut skipped: Vec<(&'static str, String)> = Vec::new();
        let mut failures: Vec<(&'static str, String)> = Vec::new();
        let mut durations: Vec<(&'static str, Duration)> = Vec::new();

        for (_, name, result, elapsed) in results {
            durations.push((name, elapsed));
            match result {
                TestResult::Pass => {
                    passed += 1;
                    println!("test {} ... ok ({})", name, format_duration(elapsed));
                }
                TestResult::Skip(reason) => {
                    println!(
                        "test {} ... skipped ({})",
                        name,
                        format_duration(elapsed)
                    );
                    if self.verbose {
                        for line in reason.lines() {
                            println!("  {line}");
                        }
                    }
                    skipped.push((name, reason));
                }
                TestResult::Fail(message) => {
                    println!(
                        "test {} ... FAILED ({})",
                        name,
                        format_duration(elapsed)
                    );
                    if self.verbose {
                        for line in message.lines() {
                            println!("  {line}");
                        }
                    }
                    failures.push((name, message));
                }
            }
        }

        self.print_summary(&failures, passed, skipped.len(), &durations)
    }

    fn print_summary(
        &self,
        failures: &[(&'static str, String)],
        passed: usize,
        skipped_count: usize,
        durations: &[(&'static str, Duration)],
    ) -> Result<bool> {
        if !failures.is_empty() {
            println!();
            println!("failures:");
            println!();
            for (name, message) in failures {
                println!("--- {name} ---");
                for line in message.lines() {
                    println!("  {line}");
                }
                println!();
            }
        }

        let failed = failures.len();
        let total = durations.iter().map(|(_, d)| d).sum::<Duration>();

        if failed == 0 {
            println!("test result: ok. {passed} passed; 0 failed; {skipped_count} skipped");
            println!("test result: total {}", format_duration(total));
            Ok(true)
        } else {
            println!(
                "test result: FAILED. {passed} passed; {failed} failed; {skipped_count} skipped"
            );
            println!("test result: total {}", format_duration(total));
            Ok(false)
        }
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
