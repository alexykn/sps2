// Crate-level pedantic settings apply
#![allow(clippy::module_name_repetitions)]

//! Parallel build orchestration system for sps2
//!
//! This module provides high-performance parallel build orchestration with:
//! - Resource-aware scheduling
//! - Work-stealing queue implementation
//! - Dependency graph execution
//! - Automatic retry with exponential backoff
//! - Progress tracking and reporting

use crate::{BuildContext, BuildResult};
use crossbeam::queue::SegQueue;
use dashmap::DashMap;
use sps2_errors::Error;
use sps2_events::{Event, EventSender};
use sps2_types::Version;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, Semaphore};
use tokio::time::sleep;

/// Maximum number of retries for failed builds
const MAX_RETRIES: u32 = 3;

/// Initial retry delay (doubles with each retry)
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Build task priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// Critical system packages
    Critical = 0,
    /// User-requested packages
    High = 1,
    /// Dependencies
    Normal = 2,
    /// Optional/suggested packages
    Low = 3,
}

/// State of a build task
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    /// Waiting in queue
    Pending,
    /// Currently building
    Running,
    /// Build completed successfully
    Completed,
    /// Build failed (with retry count)
    Failed(u32),
}

/// Resource requirements for a build task
#[derive(Debug, Clone)]
pub struct ResourceRequirements {
    /// Number of CPU cores needed
    pub cpu_cores: usize,
    /// Memory in MB
    pub memory_mb: usize,
    /// Disk I/O weight (1-100)
    pub disk_io_weight: u32,
    /// Estimated build time in seconds
    pub estimated_duration: Duration,
}

impl Default for ResourceRequirements {
    fn default() -> Self {
        Self {
            cpu_cores: 1,
            memory_mb: 512,
            disk_io_weight: 50,
            estimated_duration: Duration::from_secs(300),
        }
    }
}

/// A build task with metadata and dependencies
#[derive(Debug, Clone)]
pub struct BuildTask {
    /// Unique task ID
    pub id: String,
    /// Package name
    pub package_name: String,
    /// Package version
    pub version: Version,
    /// Task priority
    pub priority: Priority,
    /// Resource requirements
    pub resources: ResourceRequirements,
    /// Dependencies (other task IDs that must complete first)
    pub dependencies: Vec<String>,
    /// Current state
    pub state: TaskState,
    /// Build context
    pub context: BuildContext,
    /// Creation timestamp
    pub created_at: Instant,
    /// Start timestamp (when build actually started)
    pub started_at: Option<Instant>,
}

impl BuildTask {
    /// Create a new build task
    #[must_use]
    pub fn new(
        package_name: String,
        version: Version,
        context: BuildContext,
        priority: Priority,
    ) -> Self {
        let id = format!("{}-{}", package_name, version);
        Self {
            id,
            package_name,
            version,
            priority,
            resources: ResourceRequirements::default(),
            dependencies: Vec::new(),
            state: TaskState::Pending,
            context,
            created_at: Instant::now(),
            started_at: None,
        }
    }

    /// Set resource requirements
    #[must_use]
    pub fn with_resources(mut self, resources: ResourceRequirements) -> Self {
        self.resources = resources;
        self
    }

    /// Add dependencies
    #[must_use]
    pub fn with_dependencies(mut self, dependencies: Vec<String>) -> Self {
        self.dependencies = dependencies;
        self
    }
}

/// System resource tracking
#[derive(Debug)]
pub struct SystemResources {
    /// Total CPU cores available
    pub total_cpu_cores: usize,
    /// Total memory in MB
    pub total_memory_mb: usize,
    /// Total disk I/O capacity (arbitrary units)
    pub total_disk_io: u32,
}

impl SystemResources {
    /// Detect system resources
    #[must_use]
    pub fn detect() -> Self {
        let total_cpu_cores = num_cpus::get();
        // Estimate available memory (simplified - in production would use sysinfo)
        let total_memory_mb = total_cpu_cores * 2048; // 2GB per core estimate
        let total_disk_io = 100; // Arbitrary units

        Self {
            total_cpu_cores,
            total_memory_mb,
            total_disk_io,
        }
    }
}

/// Resource allocation tracking
#[derive(Debug)]
struct ResourceAllocation {
    /// Currently allocated CPU cores
    cpu: AtomicUsize,
    /// Currently allocated memory in MB
    memory: AtomicUsize,
    /// Currently allocated disk I/O weight
    disk_io: AtomicU64,
}

impl ResourceAllocation {
    fn new() -> Self {
        Self {
            cpu: AtomicUsize::new(0),
            memory: AtomicUsize::new(0),
            disk_io: AtomicU64::new(0),
        }
    }

    /// Try to allocate resources, returns true if successful
    fn try_allocate(&self, req: &ResourceRequirements, limits: &SystemResources) -> bool {
        // Try to allocate CPU
        let current_cpu = self.cpu.load(Ordering::Acquire);
        if current_cpu + req.cpu_cores > limits.total_cpu_cores {
            return false;
        }

        // Try to allocate memory
        let current_mem = self.memory.load(Ordering::Acquire);
        if current_mem + req.memory_mb > limits.total_memory_mb {
            return false;
        }

        // Try to allocate disk I/O
        let current_io = self.disk_io.load(Ordering::Acquire);
        if current_io + u64::from(req.disk_io_weight) > u64::from(limits.total_disk_io) {
            return false;
        }

        // Attempt atomic allocation
        if self
            .cpu
            .compare_exchange(
                current_cpu,
                current_cpu + req.cpu_cores,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }

        // If CPU allocation succeeded, continue with memory
        if self
            .memory
            .compare_exchange(
                current_mem,
                current_mem + req.memory_mb,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            // Rollback CPU allocation
            self.cpu.fetch_sub(req.cpu_cores, Ordering::AcqRel);
            return false;
        }

        // Finally allocate disk I/O
        if self
            .disk_io
            .compare_exchange(
                current_io,
                current_io + u64::from(req.disk_io_weight),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            // Rollback CPU and memory
            self.cpu.fetch_sub(req.cpu_cores, Ordering::AcqRel);
            self.memory.fetch_sub(req.memory_mb, Ordering::AcqRel);
            return false;
        }

        true
    }

    /// Release allocated resources
    fn release(&self, req: &ResourceRequirements) {
        self.cpu.fetch_sub(req.cpu_cores, Ordering::AcqRel);
        self.memory.fetch_sub(req.memory_mb, Ordering::AcqRel);
        self.disk_io
            .fetch_sub(u64::from(req.disk_io_weight), Ordering::AcqRel);
    }
}

/// Resource manager for tracking and allocating system resources
pub struct ResourceManager {
    /// System resource limits
    limits: SystemResources,
    /// Current allocations
    allocation: ResourceAllocation,
}

impl ResourceManager {
    /// Create a new resource manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            limits: SystemResources::detect(),
            allocation: ResourceAllocation::new(),
        }
    }

    /// Create with custom limits
    #[must_use]
    pub fn with_limits(limits: SystemResources) -> Self {
        Self {
            limits,
            allocation: ResourceAllocation::new(),
        }
    }

    /// Try to allocate resources for a task
    pub fn try_allocate(&self, requirements: &ResourceRequirements) -> bool {
        self.allocation.try_allocate(requirements, &self.limits)
    }

    /// Release resources from a completed task
    pub fn release(&self, requirements: &ResourceRequirements) {
        self.allocation.release(requirements);
    }

    /// Calculate optimal parallelism based on available resources
    #[must_use]
    pub fn optimal_parallelism(&self) -> usize {
        // Use 75% of CPU cores to leave headroom
        (self.limits.total_cpu_cores * 3 / 4).max(1)
    }

    /// Get current resource utilization percentage
    #[must_use]
    pub fn utilization(&self) -> f64 {
        let cpu_util =
            self.allocation.cpu.load(Ordering::Acquire) as f64 / self.limits.total_cpu_cores as f64;
        let mem_util = self.allocation.memory.load(Ordering::Acquire) as f64
            / self.limits.total_memory_mb as f64;
        let io_util = self.allocation.disk_io.load(Ordering::Acquire) as f64
            / f64::from(self.limits.total_disk_io);

        // Return the maximum utilization
        cpu_util.max(mem_util).max(io_util) * 100.0
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Build scheduler that manages task dependencies and resource allocation
pub struct BuildScheduler {
    /// Pending tasks by priority (work-stealing queues)
    priority_queues: Vec<Arc<SegQueue<Arc<BuildTask>>>>,
    /// All tasks by ID
    tasks: Arc<DashMap<String, Arc<BuildTask>>>,
    /// Task completion tracking
    completed: Arc<DashMap<String, BuildResult>>,
    /// Resource manager
    resources: Arc<ResourceManager>,
    /// Event sender for progress reporting
    event_sender: Option<EventSender>,
}

impl BuildScheduler {
    /// Create a new build scheduler
    #[must_use]
    pub fn new(resources: Arc<ResourceManager>) -> Self {
        // Create priority queues (one per priority level)
        let priority_queues = vec![
            Arc::new(SegQueue::new()),
            Arc::new(SegQueue::new()),
            Arc::new(SegQueue::new()),
            Arc::new(SegQueue::new()),
        ];

        Self {
            priority_queues,
            tasks: Arc::new(DashMap::new()),
            completed: Arc::new(DashMap::new()),
            resources,
            event_sender: None,
        }
    }

    /// Set event sender
    #[must_use]
    pub fn with_event_sender(mut self, sender: EventSender) -> Self {
        self.event_sender = Some(sender);
        self
    }

    /// Schedule a new task
    pub fn schedule(&self, task: BuildTask) {
        let task_arc = Arc::new(task);
        self.tasks
            .insert(task_arc.id.clone(), Arc::clone(&task_arc));

        // Check if dependencies are satisfied
        if self.are_dependencies_satisfied(&task_arc) {
            self.enqueue_task(task_arc);
        }
    }

    /// Check if all dependencies are satisfied
    fn are_dependencies_satisfied(&self, task: &BuildTask) -> bool {
        task.dependencies
            .iter()
            .all(|dep| self.completed.contains_key(dep))
    }

    /// Enqueue a task in the appropriate priority queue
    fn enqueue_task(&self, task: Arc<BuildTask>) {
        let queue_index = task.priority as usize;
        self.priority_queues[queue_index].push(task);
    }

    /// Get the next available task that can be executed
    pub fn get_next_task(&self) -> Option<Arc<BuildTask>> {
        // Try each priority queue in order
        for queue in &self.priority_queues {
            while let Some(task) = queue.pop() {
                // Check if we have resources and dependencies
                if self.are_dependencies_satisfied(&task)
                    && self.resources.try_allocate(&task.resources)
                {
                    return Some(task);
                }
                // Re-queue if we can't execute it yet
                queue.push(task);
            }
        }
        None
    }

    /// Mark a task as completed
    pub fn complete_task(&self, task_id: &str, result: BuildResult) {
        self.completed.insert(task_id.to_string(), result);

        // Check if any pending tasks can now be scheduled
        let tasks_to_check: Vec<_> = self
            .tasks
            .iter()
            .filter(|entry| {
                matches!(entry.value().state, TaskState::Pending)
                    && entry.value().dependencies.contains(&task_id.to_string())
            })
            .map(|entry| Arc::clone(entry.value()))
            .collect();

        for task in tasks_to_check {
            if self.are_dependencies_satisfied(&task) {
                self.enqueue_task(task);
            }
        }
    }

    /// Handle task failure with retry logic
    pub fn handle_failure(&self, task: &Arc<BuildTask>, error: &Error) -> bool {
        if let TaskState::Failed(retry_count) = &task.state {
            if *retry_count < MAX_RETRIES {
                // Update retry count and re-queue
                if let Some(mut entry) = self.tasks.get_mut(&task.id) {
                    let mut new_task = (**entry).clone();
                    new_task.state = TaskState::Failed(retry_count + 1);
                    *entry = Arc::new(new_task);
                }

                // Send retry event
                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(Event::Warning {
                        message: format!(
                            "Build failed for {}, retrying ({}/{})",
                            task.package_name,
                            retry_count + 1,
                            MAX_RETRIES
                        ),
                        context: Some(error.to_string()),
                    });
                }

                // Re-queue with exponential backoff
                let task_clone = Arc::clone(task);
                let queue_index = task.priority as usize;
                let queue = Arc::clone(&self.priority_queues[queue_index]);
                let retry_delay = INITIAL_RETRY_DELAY * 2u32.pow(*retry_count);

                tokio::spawn(async move {
                    sleep(retry_delay).await;
                    queue.push(task_clone);
                });

                return true; // Will retry
            }
        }

        false // No more retries
    }

    /// Get scheduler statistics
    #[must_use]
    pub fn stats(&self) -> SchedulerStats {
        let pending = self
            .priority_queues
            .iter()
            .map(|q| {
                // Count items without consuming (peek pattern)
                let items: Vec<_> = std::iter::from_fn(|| q.pop()).collect();
                let count = items.len();
                for item in items {
                    q.push(item);
                }
                count
            })
            .sum();

        SchedulerStats {
            pending_tasks: pending,
            running_tasks: self.tasks.len() - self.completed.len() - pending,
            completed_tasks: self.completed.len(),
            resource_utilization: self.resources.utilization(),
        }
    }
}

/// Scheduler statistics
#[derive(Debug, Clone)]
pub struct SchedulerStats {
    /// Number of pending tasks
    pub pending_tasks: usize,
    /// Number of running tasks
    pub running_tasks: usize,
    /// Number of completed tasks
    pub completed_tasks: usize,
    /// Resource utilization percentage
    pub resource_utilization: f64,
}

/// Build orchestrator that manages parallel builds
///
/// # Example
///
/// ```no_run
/// use sps2_builder::{Builder, BuildOrchestrator, BuildTask, BuildContext, Priority};
/// use sps2_types::Version;
/// use std::path::PathBuf;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create builder and orchestrator
/// let builder = Builder::new();
/// let orchestrator = BuildOrchestrator::new(builder);
///
/// // Schedule some builds
/// let context1 = BuildContext::new(
///     "package1".to_string(),
///     Version::parse("1.0.0")?,
///     PathBuf::from("recipes/package1.star"),
///     PathBuf::from("/tmp/output"),
/// );
///
/// let task1 = BuildTask::new(
///     "package1".to_string(),
///     Version::parse("1.0.0")?,
///     context1,
///     Priority::Normal,
/// );
///
/// orchestrator.schedule_build(task1);
///
/// // Execute all scheduled builds
/// let results = orchestrator.execute().await?;
/// println!("Built {} packages", results.len());
/// # Ok(())
/// # }
/// ```
pub struct BuildOrchestrator {
    /// Build scheduler
    scheduler: Arc<BuildScheduler>,
    /// Resource manager
    resources: Arc<ResourceManager>,
    /// Maximum parallel builds
    max_parallel: usize,
    /// Active build semaphore
    semaphore: Arc<Semaphore>,
    /// Event sender
    event_sender: Option<EventSender>,
    /// Builder instance
    builder: Arc<RwLock<crate::Builder>>,
}

impl BuildOrchestrator {
    /// Create a new build orchestrator
    #[must_use]
    pub fn new(builder: crate::Builder) -> Self {
        let resources = Arc::new(ResourceManager::new());
        let max_parallel = resources.optimal_parallelism();

        Self {
            scheduler: Arc::new(BuildScheduler::new(Arc::clone(&resources))),
            resources,
            max_parallel,
            semaphore: Arc::new(Semaphore::new(max_parallel)),
            event_sender: None,
            builder: Arc::new(RwLock::new(builder)),
        }
    }

    /// Set event sender
    #[must_use]
    pub fn with_event_sender(mut self, sender: EventSender) -> Self {
        self.event_sender = Some(sender.clone());
        self.scheduler =
            Arc::new(BuildScheduler::new(Arc::clone(&self.resources)).with_event_sender(sender));
        self
    }

    /// Schedule a build task
    pub fn schedule_build(&self, task: BuildTask) {
        // Send event
        if let Some(sender) = &self.event_sender {
            let _ = sender.send(Event::BuildStarting {
                package: task.package_name.clone(),
                version: task.version.clone(),
            });
        }

        self.scheduler.schedule(task);
    }

    /// Execute all scheduled builds
    ///
    /// Note: This executes builds sequentially due to Send constraints in the builder.
    /// For true parallel builds, the builder would need to be refactored to be Send.
    pub async fn execute(&self) -> Result<Vec<BuildResult>, Error> {
        let results = Arc::new(DashMap::new());

        loop {
            // Check if we have work to do
            let stats = self.scheduler.stats();
            if stats.pending_tasks == 0 && stats.running_tasks == 0 {
                break;
            }

            // Try to get a task
            if let Some(task) = self.scheduler.get_next_task() {
                let task_id = task.id.clone();
                let package_name = task.package_name.clone();
                let version = task.version.clone();

                // Update task state to running
                if let Some(mut entry) = self.scheduler.tasks.get_mut(&task.id) {
                    let mut new_task = (**entry).clone();
                    new_task.state = TaskState::Running;
                    new_task.started_at = Some(Instant::now());
                    *entry = Arc::new(new_task);
                }

                // Send progress event
                if let Some(sender) = &self.event_sender {
                    let _ = sender.send(Event::OperationStarted {
                        operation: format!("Building {} {}", package_name, version),
                    });
                }

                // Execute the build
                let builder_lock = self.builder.read().await;
                match builder_lock.build(task.context.clone()).await {
                    Ok(result) => {
                        // Release resources
                        self.resources.release(&task.resources);

                        // Mark as completed
                        self.scheduler.complete_task(&task_id, result.clone());
                        results.insert(task_id.clone(), result);

                        // Send completion event
                        if let Some(sender) = &self.event_sender {
                            let _ = sender.send(Event::BuildCompleted {
                                package: package_name,
                                version,
                                path: task.context.output_path(),
                            });
                        }

                        // Update task state
                        if let Some(mut entry) = self.scheduler.tasks.get_mut(&task_id) {
                            let mut new_task = (**entry).clone();
                            new_task.state = TaskState::Completed;
                            *entry = Arc::new(new_task);
                        }
                    }
                    Err(error) => {
                        // Release resources
                        self.resources.release(&task.resources);

                        // Handle failure with retry
                        if !self.scheduler.handle_failure(&Arc::clone(&task), &error) {
                            // No more retries, mark as permanently failed
                            if let Some(sender) = &self.event_sender {
                                let _ = sender.send(Event::BuildFailed {
                                    package: package_name,
                                    version,
                                    error: error.to_string(),
                                });
                            }
                        }
                    }
                }
            } else {
                // No tasks available, wait a bit
                sleep(Duration::from_millis(100)).await;
            }
        }

        // Collect results
        Ok(results.iter().map(|entry| entry.value().clone()).collect())
    }

    /// Get current statistics
    #[must_use]
    pub fn stats(&self) -> OrchestratorStats {
        let scheduler_stats = self.scheduler.stats();

        OrchestratorStats {
            scheduler: scheduler_stats,
            max_parallel: self.max_parallel,
            active_workers: self.max_parallel - self.semaphore.available_permits(),
        }
    }
}

/// Orchestrator statistics
#[derive(Debug, Clone)]
pub struct OrchestratorStats {
    /// Scheduler statistics
    pub scheduler: SchedulerStats,
    /// Maximum parallel builds
    pub max_parallel: usize,
    /// Currently active workers
    pub active_workers: usize,
}

// Implement Default for BuildTask to support tests
impl Default for BuildTask {
    fn default() -> Self {
        let temp = std::path::PathBuf::from("/tmp");
        Self {
            id: "test".to_string(),
            package_name: "test".to_string(),
            version: Version::parse("1.0.0").unwrap(),
            priority: Priority::Normal,
            resources: ResourceRequirements::default(),
            dependencies: Vec::new(),
            state: TaskState::Pending,
            context: BuildContext::new(
                "test".to_string(),
                Version::parse("1.0.0").unwrap(),
                temp.join("recipe.star"),
                temp,
            ),
            created_at: Instant::now(),
            started_at: None,
        }
    }
}
