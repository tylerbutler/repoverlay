# Completions - Manual Test

## Prerequisites

- `repoverlay` installed and on PATH

## Test Cases

### TC-01: Generate bash completions

**Steps:**

```bash
repoverlay completions bash
```

**Expected Output:**

- Shell completion script printed to stdout
- Script should be valid bash syntax

**Verify:**

```bash
repoverlay completions bash > /dev/null
echo "Exit code: $?"
# Exit code should be 0

repoverlay completions bash | head -5
# Should contain bash completion function definitions

repoverlay completions bash | grep -q "repoverlay"
echo "Contains repoverlay: $?"
# Should be 0 (completions reference the command name)
```

### TC-02: Generate zsh completions

**Steps:**

```bash
repoverlay completions zsh
```

**Expected Output:**

- Zsh completion script printed to stdout

**Verify:**

```bash
repoverlay completions zsh > /dev/null
echo "Exit code: $?"
# Exit code should be 0

repoverlay completions zsh | grep -q "repoverlay"
echo "Contains repoverlay: $?"
# Should be 0
```

### TC-03: Generate fish completions

**Steps:**

```bash
repoverlay completions fish
```

**Expected Output:**

- Fish shell completion script printed to stdout

**Verify:**

```bash
repoverlay completions fish > /dev/null
echo "Exit code: $?"
# Exit code should be 0

repoverlay completions fish | grep -q "repoverlay"
echo "Contains repoverlay: $?"
# Should be 0
```

### TC-04: Generate completions for all supported shells

**Steps:**

```bash
for shell in bash elvish fish powershell zsh; do
  repoverlay completions "$shell" > /dev/null 2>&1
  echo "$shell: exit code $?"
done
```

**Expected Output:**

- Exit code 0 for each supported shell

### TC-05: Invalid shell name

**Steps:**

```bash
repoverlay completions invalid-shell 2>&1
echo "Exit code: $?"
```

**Expected Output:**

- Error message indicating invalid shell name
- Non-zero exit code
- Should list valid shell options

## Cleanup

No cleanup needed — completions are printed to stdout only.
