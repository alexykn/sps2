# Builder Crate Improvements

This document tracks areas for improvement in the builder crate, identified during code review.

## Progress Summary

**Phase 1 (Foundation)**: 1/1 tasks completed ✓  
**Phase 2 (Architecture)**: 1/3 tasks completed ✓  
**Phase 3 (Polish)**: 0/4 tasks completed  

**Total**: 3/9 improvements completed (including 1 additional improvement)

## Completed Improvements

### Rpath Patching Clarity ✓ DONE (Additional improvement)
- **Problem**: Boolean `patch_rpaths` option was confusing - `true` meant Homebrew style, `false` meant Modern style
- **Solution**: ✓ Replaced with clear enum `RpathPatchOption` (default/absolute/skip)
  - ✓ `default` (or omitted) - Modern style with @rpath (relocatable)
  - ✓ `absolute` - Convert @rpath to absolute paths (formerly Homebrew style)
  - ✓ `skip` - No rpath patching
  - ✓ Renamed `RpathStyle::Homebrew` to `RpathStyle::Absolute` for clarity
  - ✓ Updated documentation

## Priority 1: Critical Issues

### 1. Module Organization Confusion ✓ DONE
- **Problem**: Having both `BuildStep` in yaml module and `YamlBuildStep` in recipe module is confusing
- **Impact**: Makes the codebase harder to understand and maintain
- **Solution**: ✓ Renamed types for clarity
  - ✓ Renamed `recipe::model::BuildStep` to `ParsedStep` (YAML representation)
  - ✓ Kept `yaml::BuildStep` as `BuildStep` (execution representation)
  - ✓ Updated lib.rs exports to use `ParsedStep` instead of `BuildStep as YamlBuildStep`
  - Flow is now clear: YAML → ParsedStep → BuildStep → execute

### 2. BuilderApi Design ✓ DONE
- **Problem**: The `BuilderApi` in core/api.rs feels like a grab bag of unrelated functionality
- **Impact**: Unclear responsibilities, hard to test, methods like `install()` just return success
- **Solution**:
  - Split into focused interfaces: `BuildCommands`, `BuildQueries`, `BuildEnvironment`
  - Complete the unimplemented methods or remove them
  - Clarify the relationship between Builder, BuilderApi, and BuildEnvironment
- **Progress**:
  - ✓ Created stage-based architecture with separate types for each stage (SourceStep, BuildCommand, PostStep, EnvironmentStep)
  - ✓ Created stage-specific executors in `stages/executors.rs`
  - ✓ Updated BuildPlan to use new stage-specific types
  - ✓ Refactored utils/executor.rs to use stage-specific executors
  - ✓ Added validation layer between ParsedStep and stage-specific types
    - ✓ Created validation module with secure command parsing
    - ✓ Implemented security rules to block dangerous commands (sudo, rm -rf /, etc.)
    - ✓ Added path validation to prevent traversal attacks
    - ✓ Added URL validation to block suspicious URLs
    - ✓ Integrated validation into BuildPlan::from_yaml
  - ✓ Removed old monolithic execute_build_step function (replaced by stage executors)

## Priority 2: Architecture Improvements

### 4. Platform Abstraction Layer
- **Problem**: Hardcoded macOS assumptions throughout ("macOS ARM only", RPATH handling)
- **Impact**: Limits portability, makes cross-platform support difficult
- **Solution**:
  - Create a `platform` module for OS-specific concerns
  - Abstract RPATH handling, code signing, file permissions
  - Make cross-compilation a first-class concern

### 5. Caching Implementation
- **Problem**: `ContentAddressedStore` and `ArtifactCache` duplicate functionality
- **Impact**: Complexity, potential bugs, unclear cache eviction
- **Solution**:
  - Consolidate into a single, well-designed cache system (that uses what the project provides in terms of cas in the state and store crates...)
  - Integrate statistics tracking properly
  - Implement clear eviction policies (not just LRU)
  - Add cache warming/seeding capabilities

### 6. Configuration Management
- **Problem**: Too many config types (BuildConfig, SbomConfig, CompressionConfig, etc.)
- **Impact**: Configuration sprawl, hard to manage settings
- **Solution**:
  - Create a unified configuration system with sections
  - Use a builder pattern or config facade
  - Consider TOML/YAML for config files

## Priority 3: Code Quality

### 7. Timeout Handling
- **Problem**: Incomplete timeout implementation in staged executor
- **Impact**: Builds can hang indefinitely
- **Solution**:
  - Add per-stage timeout configuration
  - Implement proper timeout handling with cleanup
  - Add progress tracking for long operations

### 8. Documentation Gaps
- **Problem**: Mechanical docs ("Returns an error if..."), no high-level overview
- **Impact**: Hard for new contributors to understand the system
- **Solution**:
  - Add architecture.md explaining the build flow
  - Include usage examples
  - Document the artifact QA pipeline thoroughly
  - Add inline examples for public APIs

### 9. Code Smells
- **Problem**: Commented out code, TODOs, magic values
- **Impact**: Technical debt, unclear intentions
- **Solution**:
  - Replace commented cleanup with debug flag
  - Check TODOs for implementation, if implemented remove, if not determine their value to the project and plan implementation
  - Extract magic values to constants
  - Add configuration for hardcoded paths

## Implementation Plan

### Phase 1: Foundation
1. ✓ Fix module organization confusion - COMPLETED

### Phase 2: Architecture
1. Implement platform abstraction layer
2. Redesign BuilderApi into focused components ✓ DONE
   - ✓ Stage-based architecture created
   - ✓ Added comprehensive validation layer with security checks
   - ✓ Removed monolithic execute_build_step function
3. Consolidate caching implementation

### Phase 3: Polish
1. Improve documentation
2. Fix timeout handling
3. Clean up code smells
4. Unify configuration management

## Success Metrics
- [ ] All TODOs covered
- [x] Clear module boundaries with no circular dependencies (ParsedStep → BuildStep flow)
- [ ] Platform-specific code isolated in platform module
- [ ] Comprehensive documentation for all public APIs

## Notes
- These improvements should be done incrementally
- Each change should maintain backward compatibility where possible
- Focus on the most impactful changes first (Priority 1)
