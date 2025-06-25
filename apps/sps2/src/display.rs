//! Output rendering and formatting

use comfy_table::{presets::UTF8_FULL, Attribute, Cell, Color, ContentArrangement, Table};
use console::{Style, Term};
use sps2_ops::{
    AuditReport, BuildReport, HealthCheck, HealthStatus, InstallReport, IssueSeverity,
    OperationResult, PackageInfo, PackageStatus, SearchResult, Severity, StateInfo, VulnDbStats,
};
use sps2_types::ColorChoice;
use std::io;

/// Output renderer for CLI results
#[derive(Clone)]
pub struct OutputRenderer {
    /// Use JSON output format
    json_output: bool,
    /// Color configuration
    color_choice: ColorChoice,
    /// Terminal instance
    term: Term,
}

impl OutputRenderer {
    /// Create new output renderer
    pub fn new(json_output: bool, color_choice: ColorChoice) -> Self {
        Self {
            json_output,
            color_choice,
            term: Term::stdout(),
        }
    }

    /// Render operation result
    pub fn render_result(&self, result: &OperationResult) -> io::Result<()> {
        if self.json_output {
            self.render_json(result)
        } else {
            self.render_table(result)
        }
    }

    /// Render as JSON
    fn render_json(&self, result: &OperationResult) -> io::Result<()> {
        let json = result.to_json().map_err(io::Error::other)?;
        println!("{json}");
        Ok(())
    }

    /// Render as formatted table
    fn render_table(&self, result: &OperationResult) -> io::Result<()> {
        match result {
            OperationResult::PackageList(packages) => self.render_package_list(packages),
            OperationResult::PackageInfo(info) => self.render_package_info(info),
            OperationResult::SearchResults(results) => self.render_search_results(results),
            OperationResult::InstallReport(report) => self.render_install_report(report),
            OperationResult::BuildReport(report) => self.render_build_report(report),
            OperationResult::StateInfo(info) => self.render_state_info(info),
            OperationResult::StateHistory(history) => self.render_state_history(history),
            OperationResult::HealthCheck(health) => self.render_health_check(health),
            OperationResult::Success(message) => self.render_success_message(message),
            OperationResult::Report(report) => self.render_op_report(report),
            OperationResult::VulnDbStats(stats) => self.render_vulndb_stats(stats),
            OperationResult::AuditReport(report) => self.render_audit_report(report),
            OperationResult::VerificationResult(result) => self.render_verification_result(result),
        }
    }

    fn render_verification_result(&self, result: &sps2_ops::VerificationResult) -> io::Result<()> {
        if self.json_output {
            println!("{}", serde_json::to_string_pretty(result).map_err(io::Error::other)?);
            return Ok(());
        }

        if result.is_valid {
            println!("[OK] State verification passed.");
        } else {
            println!("[ERROR] State verification failed with {} discrepancies:", result.discrepancies.len());
            for discrepancy in &result.discrepancies {
                println!("  - {:?}", discrepancy);
            }
        }

        Ok(())
    }

    /// Render package list
    fn render_package_list(&self, packages: &[PackageInfo]) -> io::Result<()> {
        if packages.is_empty() {
            println!("No packages installed.");
            return Ok(());
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);

        // Add headers
        table.set_header(vec![
            Cell::new("Package").add_attribute(Attribute::Bold),
            Cell::new("Version").add_attribute(Attribute::Bold),
            Cell::new("Status").add_attribute(Attribute::Bold),
            Cell::new("Description").add_attribute(Attribute::Bold),
        ]);

        // Add package rows
        for package in packages {
            let status_cell = self.format_package_status(&package.status);
            let version_str = package
                .version
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_else(|| "-".to_string());

            table.add_row(vec![
                Cell::new(&package.name),
                Cell::new(version_str),
                status_cell,
                Cell::new(package.description.as_deref().unwrap_or("-")),
            ]);
        }

        println!("{table}");
        Ok(())
    }

    /// Render package information
    fn render_package_info(&self, info: &PackageInfo) -> io::Result<()> {
        println!("{}", self.style_package_name(&info.name));
        println!();

        // Basic information
        if let Some(description) = &info.description {
            println!("Description: {description}");
        }

        if let Some(version) = &info.version {
            println!("Installed:   {version}");
        }

        if let Some(available) = &info.available_version {
            println!("Available:   {available}");
        }

        println!(
            "Status:      {}",
            self.format_package_status_text(&info.status)
        );

        if let Some(license) = &info.license {
            println!("License:     {license}");
        }

        if let Some(homepage) = &info.homepage {
            println!("Homepage:    {homepage}");
        }

        // Dependencies
        if !info.dependencies.is_empty() {
            println!();
            println!("Dependencies:");
            for dep in &info.dependencies {
                println!("  • {dep}");
            }
        }

        // Size information
        if let Some(size) = info.size {
            println!();
            println!("Size:        {}", format_size(size));
        }

        Ok(())
    }

    /// Render search results
    fn render_search_results(&self, results: &[SearchResult]) -> io::Result<()> {
        if results.is_empty() {
            println!("No packages found.");
            return Ok(());
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);

        table.set_header(vec![
            Cell::new("Package").add_attribute(Attribute::Bold),
            Cell::new("Version").add_attribute(Attribute::Bold),
            Cell::new("Installed").add_attribute(Attribute::Bold),
            Cell::new("Description").add_attribute(Attribute::Bold),
        ]);

        for result in results {
            let installed_text = if result.installed { "Yes" } else { "No" };
            let installed_cell = if result.installed {
                Cell::new(installed_text).fg(Color::Green)
            } else {
                Cell::new(installed_text)
            };

            table.add_row(vec![
                Cell::new(&result.name),
                Cell::new(result.version.to_string()),
                installed_cell,
                Cell::new(result.description.as_deref().unwrap_or("-")),
            ]);
        }

        println!("{table}");
        Ok(())
    }

    /// Render installation report
    fn render_install_report(&self, report: &InstallReport) -> io::Result<()> {
        let total_changes = report.installed.len() + report.updated.len() + report.removed.len();

        if total_changes == 0 {
            println!("No changes made.");
            return Ok(());
        }

        // Summary
        println!("Installation Summary");
        println!();

        if !report.installed.is_empty() {
            println!("Installed ({}):", report.installed.len());
            for change in &report.installed {
                let version = change
                    .to_version
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                println!("  • {} {}", change.name, version);
            }
            println!();
        }

        if !report.updated.is_empty() {
            println!("Updated ({}):", report.updated.len());
            for change in &report.updated {
                let from = change
                    .from_version
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let to = change
                    .to_version
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                println!("  • {} {} → {}", change.name, from, to);
            }
            println!();
        }

        if !report.removed.is_empty() {
            println!("Removed ({}):", report.removed.len());
            for change in &report.removed {
                let version = change
                    .from_version
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                println!("  • {} {}", change.name, version);
            }
            println!();
        }

        println!("Completed in {}ms", report.duration_ms);
        println!("State: {}", report.state_id);

        Ok(())
    }

    /// Render build report
    fn render_build_report(&self, report: &BuildReport) -> io::Result<()> {
        println!("Build Summary");
        println!();
        println!("Package:  {} {}", report.package, report.version);
        println!("Output:   {}", report.output_path.display());
        println!("Duration: {}ms", report.duration_ms);
        println!(
            "SBOM:     {}",
            if report.sbom_generated { "Yes" } else { "No" }
        );

        Ok(())
    }

    /// Render state information
    fn render_state_info(&self, info: &StateInfo) -> io::Result<()> {
        println!("State Information");
        println!();
        println!("ID:       {}", info.id);
        println!("Current:  {}", if info.current { "Yes" } else { "No" });
        println!(
            "Created:  {}",
            info.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!("Operation: {}", info.operation);
        println!("Packages: {}", info.package_count);

        if let Some(parent) = info.parent_id {
            println!("Parent:   {parent}");
        }

        if !info.changes.is_empty() {
            println!();
            println!("Changes:");
            for change in &info.changes {
                match change.change_type {
                    sps2_ops::ChangeType::Install => {
                        println!(
                            "  + {} {}",
                            change.package,
                            change.new_version.as_ref().unwrap()
                        );
                    }
                    sps2_ops::ChangeType::Update => {
                        println!(
                            "  ~ {} {} → {}",
                            change.package,
                            change.old_version.as_ref().unwrap(),
                            change.new_version.as_ref().unwrap()
                        );
                    }
                    sps2_ops::ChangeType::Remove => {
                        println!(
                            "  - {} {}",
                            change.package,
                            change.old_version.as_ref().unwrap()
                        );
                    }
                    sps2_ops::ChangeType::Downgrade => {
                        println!(
                            "  ↓ {} {} → {}",
                            change.package,
                            change.old_version.as_ref().unwrap(),
                            change.new_version.as_ref().unwrap()
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Render state history
    fn render_state_history(&self, history: &[StateInfo]) -> io::Result<()> {
        if history.is_empty() {
            println!("No state history found.");
            return Ok(());
        }

        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);

        table.set_header(vec![
            Cell::new("State ID").add_attribute(Attribute::Bold),
            Cell::new("Current").add_attribute(Attribute::Bold),
            Cell::new("Operation").add_attribute(Attribute::Bold),
            Cell::new("Created").add_attribute(Attribute::Bold),
            Cell::new("Packages").add_attribute(Attribute::Bold),
        ]);

        for state in history {
            let current_cell = if state.current {
                Cell::new("*")
                    .fg(Color::Green)
                    .add_attribute(Attribute::Bold)
            } else {
                Cell::new("")
            };

            table.add_row(vec![
                Cell::new(state.id.to_string()),
                current_cell,
                Cell::new(&state.operation),
                Cell::new(state.timestamp.format("%Y-%m-%d %H:%M").to_string()),
                Cell::new(state.package_count.to_string()),
            ]);
        }

        println!("{table}");
        Ok(())
    }

    /// Render health check results
    fn render_health_check(&self, health: &HealthCheck) -> io::Result<()> {
        let overall_icon = if health.healthy { "[OK]" } else { "[ERROR]" };
        println!("{overall_icon} System Health Check");
        println!();

        // Component status table
        let mut table = Table::new();
        table
            .load_preset(UTF8_FULL)
            .set_content_arrangement(ContentArrangement::Dynamic);

        table.set_header(vec![
            Cell::new("Component").add_attribute(Attribute::Bold),
            Cell::new("Status").add_attribute(Attribute::Bold),
            Cell::new("Duration").add_attribute(Attribute::Bold),
            Cell::new("Message").add_attribute(Attribute::Bold),
        ]);

        for component in health.components.values() {
            let status_cell = match component.status {
                HealthStatus::Healthy => Cell::new("Healthy").fg(Color::Green),
                HealthStatus::Warning => Cell::new("Warning").fg(Color::Yellow),
                HealthStatus::Error => Cell::new("Error").fg(Color::Red),
            };

            table.add_row(vec![
                Cell::new(&component.name),
                status_cell,
                Cell::new(format!("{}ms", component.check_duration_ms)),
                Cell::new(&component.message),
            ]);
        }

        println!("{table}");

        // Issues
        if !health.issues.is_empty() {
            println!();
            println!("Issues Found:");

            for issue in &health.issues {
                let severity_icon = match issue.severity {
                    IssueSeverity::Low => "[INFO]",
                    IssueSeverity::Medium => "[WARN]",
                    IssueSeverity::High => "[HIGH]",
                    IssueSeverity::Critical => "[CRITICAL]",
                };

                println!(
                    "{severity_icon} {} ({}): {}",
                    issue.component,
                    format!("{:?}", issue.severity).to_lowercase(),
                    issue.description
                );

                if let Some(suggestion) = &issue.suggestion {
                    println!("   {suggestion}");
                }
                println!();
            }
        }

        Ok(())
    }

    /// Render success message
    fn render_success_message(&self, message: &str) -> io::Result<()> {
        println!("{message}");
        Ok(())
    }

    /// Render operation report
    fn render_op_report(&self, report: &sps2_ops::OpReport) -> io::Result<()> {
        let icon = if report.success { "[OK]" } else { "[ERROR]" };
        println!("{icon} {} Report", report.operation);
        println!();
        println!("Summary: {}", report.summary);
        println!("Duration: {}ms", report.duration_ms);

        if !report.changes.is_empty() {
            println!();
            println!("Changes:");
            for change in &report.changes {
                match change.change_type {
                    sps2_ops::ChangeType::Install => {
                        println!(
                            "  + {} {}",
                            change.package,
                            change.new_version.as_ref().unwrap()
                        );
                    }
                    sps2_ops::ChangeType::Update => {
                        println!(
                            "  ~ {} {} → {}",
                            change.package,
                            change.old_version.as_ref().unwrap(),
                            change.new_version.as_ref().unwrap()
                        );
                    }
                    sps2_ops::ChangeType::Remove => {
                        println!(
                            "  - {} {}",
                            change.package,
                            change.old_version.as_ref().unwrap()
                        );
                    }
                    sps2_ops::ChangeType::Downgrade => {
                        println!(
                            "  ↓ {} {} → {}",
                            change.package,
                            change.old_version.as_ref().unwrap(),
                            change.new_version.as_ref().unwrap()
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Format package status as colored cell
    fn format_package_status(&self, status: &PackageStatus) -> Cell {
        match status {
            PackageStatus::Installed => Cell::new("Installed").fg(Color::Green),
            PackageStatus::Outdated => Cell::new("Outdated").fg(Color::Yellow),
            PackageStatus::Available => Cell::new("Available").fg(Color::Blue),
            PackageStatus::Local => Cell::new("Local").fg(Color::Magenta),
        }
    }

    /// Format package status as text
    fn format_package_status_text(&self, status: &PackageStatus) -> String {
        match status {
            PackageStatus::Installed => "Installed".to_string(),
            PackageStatus::Outdated => "Update available".to_string(),
            PackageStatus::Available => "Available".to_string(),
            PackageStatus::Local => "Local".to_string(),
        }
    }

    /// Style package name
    fn style_package_name(&self, name: &str) -> String {
        if self.supports_color() {
            Style::new().bold().apply_to(name).to_string()
        } else {
            name.to_string()
        }
    }

    /// Check if color output is supported
    fn supports_color(&self) -> bool {
        match self.color_choice {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => self.term.features().colors_supported(),
        }
    }

    /// Render vulnerability database statistics
    fn render_vulndb_stats(&self, stats: &VulnDbStats) -> io::Result<()> {
        println!("Vulnerability Database Statistics");
        println!();
        println!("Total Vulnerabilities: {}", stats.vulnerability_count);

        if let Some(last_updated) = &stats.last_updated {
            println!(
                "Last Updated:         {}",
                last_updated.format("%Y-%m-%d %H:%M:%S UTC")
            );
        } else {
            println!("Last Updated:         Never");
        }

        println!("Database Size:        {}", format_size(stats.database_size));

        if !stats.severity_breakdown.is_empty() {
            println!();
            println!("Severity Breakdown:");

            let severities = ["critical", "high", "medium", "low"];
            for severity in &severities {
                if let Some(count) = stats.severity_breakdown.get(*severity) {
                    let icon = match *severity {
                        "critical" => "[CRITICAL]",
                        "high" => "[HIGH]",
                        "medium" => "[WARN]",
                        "low" => "[INFO]",
                        _ => "•",
                    };
                    println!("  {icon} {severity:8}: {count:6}");
                }
            }
        }

        Ok(())
    }

    /// Render audit report
    fn render_audit_report(&self, report: &AuditReport) -> io::Result<()> {
        println!("Security Audit Report");
        println!();
        println!(
            "Scan Time:     {}",
            report.scan_timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        );
        println!("Packages:      {}", report.summary.packages_scanned);
        println!("Vulnerabilities: {}", report.summary.total_vulnerabilities);

        if report.summary.total_vulnerabilities > 0 {
            println!();
            println!("Severity Breakdown:");
            println!("  Critical: {}", report.summary.critical_count);
            println!("  High:     {}", report.summary.high_count);
            println!("  Medium:   {}", report.summary.medium_count);
            println!("  Low:      {}", report.summary.low_count);

            println!();
            println!(
                "Vulnerable Packages ({}):",
                report.summary.vulnerable_packages
            );

            for audit in &report.package_audits {
                if !audit.vulnerabilities.is_empty() {
                    println!();
                    println!(
                        "{} v{}",
                        self.style_package_name(&audit.package_name),
                        audit.package_version
                    );

                    // Group vulnerabilities by severity
                    let mut critical_vulns = Vec::new();
                    let mut high_vulns = Vec::new();
                    let mut medium_vulns = Vec::new();
                    let mut low_vulns = Vec::new();

                    for vuln_match in &audit.vulnerabilities {
                        match vuln_match.vulnerability.severity {
                            Severity::Critical => critical_vulns.push(vuln_match),
                            Severity::High => high_vulns.push(vuln_match),
                            Severity::Medium => medium_vulns.push(vuln_match),
                            Severity::Low => low_vulns.push(vuln_match),
                        }
                    }

                    // Display vulnerabilities by severity
                    for (severity, icon, vulns) in [
                        ("CRITICAL", "[CRITICAL]", critical_vulns),
                        ("HIGH", "[HIGH]", high_vulns),
                        ("MEDIUM", "[WARN]", medium_vulns),
                        ("LOW", "[INFO]", low_vulns),
                    ] {
                        for vuln_match in vulns {
                            let vuln = &vuln_match.vulnerability;
                            println!(
                                "   {} {} {} - {}",
                                icon, severity, vuln.cve_id, vuln.summary
                            );

                            if let Some(score) = vuln.cvss_score {
                                println!("      CVSS Score: {:.1}", score);
                            }

                            if !vuln.fixed_versions.is_empty() {
                                println!("      Fixed in: {}", vuln.fixed_versions.join(", "));
                            }
                        }
                    }
                }
            }
        } else {
            println!();
            println!("No vulnerabilities found!");
        }

        Ok(())
    }
}

/// Format byte size in human readable format
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{size:.0} {}", UNITS[unit_index])
    } else {
        format!("{size:.1} {}", UNITS[unit_index])
    }
}
