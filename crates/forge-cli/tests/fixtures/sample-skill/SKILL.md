---
name: Sample Skill
version: 1.0.0
description: Fixture skill used by F-590 integration tests
tools:
  - shell
---

This skill exists only as a fixture for the `forge skill install` integration
test. It validates that a folder containing `SKILL.md` round-trips through
the local-path resolver, the install step, and F-589 discovery.
