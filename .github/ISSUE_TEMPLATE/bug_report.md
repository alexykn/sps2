---
name: Bug report
about: Create a report to help us improve
title: ''
labels: ''
assignees: ''

---

### Bug Report

**Describe the bug**
A clear and concise description of what the bug is. Please include the primary error message you received.

**To Reproduce**
Please provide the exact steps to reproduce the behavior.

1.  Command(s) run (e.g., `sps2 install "jq>=1.6"` or `sps2 build my-package.star`):
    ```sh
    # paste command here
    ```

2.  Full terminal output. Please run the command with the `--debug` flag and paste the complete output below.
    ```sh
    # paste full debug output here
    ```

3.  If this is a build-related bug (`sps2 build`), please provide the full `recipe.star` file.
    ```python
    # paste recipe.star here
    ```

**Expected behavior**
A clear and concise description of what you expected to happen.

**Environment (please complete the following information):**
This information is critical for diagnosing the issue. Please paste the output of the following commands.

* **`sps2 --version`**:
    ```
    (paste output here)
    ```
* **`sps2 check-health --json`**:
    ```
    (paste output here)
    ```
* **macOS Version**: (e.g., macOS Sonoma 14.4.1)
* **Apple Silicon Chip**: (e.g., M1, M2 Pro, M3 Max)
* **Shell**: (e.g., zsh, bash)

**Additional context**
Add any other context about the problem here. This could include:
* Relevant sections of your `~/.config/sps2/config.toml` file.
* Whether this is a regression (i.e., it worked in a previous version).
* Any unusual setup in your environment.
