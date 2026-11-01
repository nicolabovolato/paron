# Contributing to Paron

## Before you start

Open an issue before you start a large change. Then the design is agreed before you do
the work.

## The Contributor License Agreement

You must agree to the [Contributor License Agreement](CLA.md) (CLA) before a
contribution can be merged. This applies to code and to documentation. It also applies
to the utility crates that have the MIT license.

In short:

- You keep the copyright in your work.
- You give the maintainer the right to use your work and to license it under any terms.

Paron is source-available under the [PolyForm Shield License 1.0.0](LICENSE). Without this right,
one merged contribution can prevent a later change of license.

On your first pull request, a bot asks you to agree with a comment. It then records
your GitHub account and the pull request on the `cla-signatures` branch. You do this
only one time.

If your employer owns the work that you make, get permission from your employer before
you sign. Section 4 of the CLA gives the details.

## Development

You need [Bun](https://bun.sh), because the build also builds the web UI, and Docker,
because some tests start a container.

CI runs the formatter, the linters and the tests on each pull request, and all of them
must pass. The commands are in [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## Change files

A change that users see needs a change file. In the repository root, run:

```sh
bunx covector add
```

The command asks for the package that you changed, for the type of change (patch, minor
or major) and for a summary. It writes a file in `.changes`. Commit that file with your
change. The next release gets its version and its changelog from these files.