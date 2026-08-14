# Review an agent patch in CI

`deed review` compares two checked module sets. A pull request already names
those sets: its base commit and its head commit. Build detached worktrees from
those commits rather than reading the runner's working directory, so generated
and untracked files cannot enter the receipt.

For example, this checked module adds authority that
`--deny-new-authority` would stop if it appeared in the head tree alone.

Playground: [open](https://deed-lang.github.io/play/)

```deed review-ci-subject
module billing

effect Store {
    fn write(value: Int) -> ()
}

fn sync(value: Int) -> ()
  uses
    Store.write,
{
    Store.write(value)
}
```

Save this as `.github/workflows/deed-review.yml`:

```yaml
name: Deed review

on:
  pull_request:

permissions:
  contents: read

env:
  # Change this when the checked Deed module set is not the repository root.
  DEED_PATH: .

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          fetch-depth: 0
          ref: ${{ github.event.pull_request.head.sha }}

      - name: Install Deed
        env:
          DEED_VERSION: 0.2.11
          DEED_INSTALL_DIR: ${{ runner.temp }}/deed-bin
        run: |
          curl -fsSL https://raw.githubusercontent.com/deed-lang/deed/v0.2.11/install.sh | sh
          echo "$DEED_INSTALL_DIR" >> "$GITHUB_PATH"

      - name: Materialize the reviewed trees
        env:
          BASE_SHA: ${{ github.event.pull_request.base.sha }}
          HEAD_SHA: ${{ github.event.pull_request.head.sha }}
        run: |
          git worktree add --detach "$RUNNER_TEMP/deed-before" "$BASE_SHA"
          git worktree add --detach "$RUNNER_TEMP/deed-after" "$HEAD_SHA"

      - name: Review the patch
        run: |
          deed review \
            --before "$RUNNER_TEMP/deed-before/$DEED_PATH" \
            --after "$RUNNER_TEMP/deed-after/$DEED_PATH" \
            --deny-new-authority \
            --deny-weaker-promises \
            --deny-new-guarded
```

The installer is read from the same tag as the requested binary. It verifies
the downloaded release asset against that release's checksum list, installs
one file under the runner's temporary directory, and adds no repository
secret. `pull_request` therefore remains safe for contributions from forks.

The three policies are independent. Remove a flag only when that kind of
evidence is informational for the repository. Keep the command as one
invocation: three separate reviews can observe three different input sets and
produce three receipts for one patch.

If the repository contains generated but untracked Deed files, no exclusion is
needed: a detached worktree starts with only files committed at the named SHA.
Set `DEED_PATH` to the directory containing the checked module set when the
repository also commits deliberately invalid conformance fixtures, generated
sources, or several independent Deed projects. The same relative path is used
under `deed-before` and `deed-after`.

The first PR that introduces Deed establishes a baseline, so land that checked
module set before enabling deny policies. With no previous Deed program, every
declared authority is correctly new; silently treating it as old would make
the first receipt a lie.