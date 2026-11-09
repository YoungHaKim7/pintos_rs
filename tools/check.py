#!/usr/bin/env python3
"""Runs threads tests in QEMU and checks them like C pintos' make check.

Reimplements the essential semantics of tests/tests.pm:
  - common_checks: boot/complete/shutdown messages, no PANIC/FAIL/
    TIMEOUT, no triple fault.
  - check_expected: exact match against the heredoc block in the
    test's .ck file.
  - check_alarm (alarm.pm): the product= sequence must be the
    sorted expected products, in order.

Usage:
  tools/check.py TEST [TEST...]   # run and check specific tests
  tools/check.py --all            # run the whole threads suite
"""

import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
KAIST = os.path.join(os.path.dirname(ROOT), "pintos-kaist")
CK_DIR = os.path.join(KAIST, "tests", "threads")

# check_alarm(iterations) tests: name -> iterations.
ALARM_TESTS = {"alarm-single": 1, "alarm-multiple": 7}

# priority-fifo.ck checks same round-robin order across iterations
# rather than a fixed expected block.
FIFO_TEST = "priority-fifo"

# ------------------------------------------------------------------ #
# userprog suite (tests/userprog/Make.tests)                          #
# ------------------------------------------------------------------ #
UPROG_DIR = os.path.join(ROOT, "build", "userprogs")
SAMPLE_TXT = os.path.join(KAIST, "tests", "userprog", "sample.txt")

USERPROG_ARGS = {
    "args-single": "onearg",
    "args-multiple": "some arguments for you!",
    "args-many": "a b c d e f g h i j k l m n o p q r s t u v",
    "args-dbl-space": "two  spaces!",
    "multi-recurse": "15",
}

USERPROG_PUTFILES = {}
for _t in [
    "open-normal", "open-boundary", "open-twice", "close-normal", "close-twice",
    "read-normal", "read-bad-ptr", "read-boundary", "read-zero", "write-normal",
    "write-bad-ptr", "write-boundary", "write-zero", "fork-read", "fork-close",
    "exec-read", "multi-child-fd",
]:
    USERPROG_PUTFILES[_t] = ["sample.txt"]
for _t, _children in {
    "exec-boundary": ["child-simple"], "exec-once": ["child-simple"],
    "wait-simple": ["child-simple"], "wait-twice": ["child-simple"],
    "exec-arg": ["child-args"], "multi-child-fd": ["child-close"],
    "wait-killed": ["child-bad"], "rox-child": ["child-rox"],
    "rox-multichild": ["child-rox"], "exec-read": ["child-read"],
}.items():
    USERPROG_PUTFILES.setdefault(_t, []).extend(_children)


def run_qemu_userprog(test: str) -> str:
    cmd = ["python3", os.path.join(HERE, "pintos.py"),
           "--disk", "build/os.dsk", "--fs-disk", "4",
           "--timeout", "90"]
    cmd += ["-p", os.path.join(UPROG_DIR, test)]
    for extra in USERPROG_PUTFILES.get(test, []):
        host = SAMPLE_TXT if extra == "sample.txt" else os.path.join(UPROG_DIR, extra)
        cmd += ["-p", host]
    cmd += ["--", "-q", "-f", "run", f"{test} {USERPROG_ARGS.get(test, '')}".strip()]
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    return proc.stdout + proc.stderr

TIMEOUT = {"alarm-priority": 120, "alarm-multiple": 120}


class TestFailure(Exception):
    pass


def run_qemu(test: str) -> str:
    cmd = [
        "python3", os.path.join(HERE, "pintos.py"),
        "--disk", "build/os.dsk",
        "--timeout", str(TIMEOUT.get(test, 90)),
        "--", "-q", "-threads-tests", "run", test,
    ]
    proc = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    return proc.stdout + proc.stderr


def common_checks(out: str) -> None:
    if "PANIC" in out:
        line = next(l for l in out.splitlines() if "PANIC" in l)
        raise TestFailure(f"kernel panic: {line[line.index('PANIC')][:120]}")
    if "FAIL" in out:
        line = next(l for l in out.splitlines() if "FAIL" in l)
        raise TestFailure(f"FAIL keyword: {line}")
    if "TIMEOUT" in out:
        raise TestFailure("TIMEOUT")
    if out.count("Pintos booting") != 1:
        raise TestFailure(f"rebooted {out.count('Pintos booting') - 1} times (triple fault)")
    if "Pintos booting with" not in out:
        raise TestFailure('no "Pintos booting" message')
    if "Boot complete" not in out:
        raise TestFailure('no "Boot complete" message')
    if not re.search(r"Timer: \d+ ticks", out):
        raise TestFailure('no "Timer: N ticks" message')
    if "Powering off" not in out:
        raise TestFailure('no "Powering off" message')


def extract_body(out: str, test: str) -> list[str]:
    """The (name) begin .. (name) end block, inclusive.

    The test program picks its own display name (e.g. all args-*
    tests print as "(args)"), so match generically: the first
    begin marker's name, and the last end marker with that name.
    """
    lines = out.splitlines()
    begin_name = None
    for i, l in enumerate(lines):
        m = re.match(r"^\(([\w.-]+)\) begin$", l)
        if m:
            begin_name = m.group(1)
            begin = i
            break
    if begin_name is None:
        raise TestFailure("missing (test) begin/end markers")
    end = None
    for i, l in enumerate(lines):
        if l == f"({begin_name}) end":
            end = i
    if end is None:
        raise TestFailure("missing (test) end marker")
    return lines[begin : end + 1]


def ck_expected(test: str) -> tuple[list[str], set[str]]:
    """Expected heredoc lines and options from the test's .ck file."""
    with open(os.path.join(CK_DIR, f"{test}.ck")) as f:
        text = f.read()
    m = re.search(r"check_expected\s*\((.*?)\[$", text, re.M | re.S)
    options = set(re.findall(r"(IGNORE_USER_FAULTS|IGNORE_EXIT_CODES)", text))
    m = re.search(r"<<'EOF'\];?\n(.*?)\nEOF\n", text, re.S) or re.search(
        r"<<'EOF'[^\n]*\n(.*?)\nEOF\n", text, re.S
    )
    if not m:
        raise TestFailure(f".ck for {test} has no plain EOF block")
    return m.group(1).splitlines(), options


def apply_filters(body: list[str], options: set[str]) -> list[str]:
    """The tests.pm compare_output output filters."""
    out = body
    if "IGNORE_EXIT_CODES" in options:
        out = [l for l in out if not re.match(r"^[a-zA-Z0-9-_]+: exit\(-?\d+\)$", l)]
    if "IGNORE_USER_FAULTS" in options:
        out = [
            l
            for l in out
            if not re.match(r"^Page fault at.*in user context\.$", l)
            and not re.search(r": dying due to interrupt 0x0e \(.*\)\.$", l)
            and not re.match(r"^Interrupt 0x0e \(.*\) at eip=", l)
            and not re.match(r"^ cr2=.* error=.*", l)
            and not re.match(r"^ eax=.* ebx=.* ecx=.* edx=.*", l)
            and not re.match(r"^ esi=.* edi=.* esp=.* ebp=.*", l)
            and not re.match(r"^ cs=.* ds=.* es=.* ss=.*", l)
        ]
    return out


def check_expected(test: str, body: list[str]) -> None:
    expected, options = ck_expected(test)
    body = apply_filters(body, options)
    for i, (got, want) in enumerate(zip(body, expected)):
        if got != want:
            raise TestFailure(f"line {i}: got `{got}', want `{want}'")
    if len(body) != len(expected):
        raise TestFailure(f"{len(body)} output lines, {len(expected)} expected")


def check_alarm(test: str, iterations: int, body: list[str]) -> None:
    products = sorted(
        (i + 1) * (t + 1) * 10 for i in range(iterations) for t in range(5)
    )
    for line in body:
        if re.search(r"out of order", line, re.I):
            raise TestFailure(line)
        m = re.search(r"product=(\d+)$", line)
        if not m:
            continue
        p = int(m.group(1))
        if not products:
            raise TestFailure("too many wakeups")
        q = products.pop(0)
        if p != q:
            raise TestFailure(f"out of order wakeups ({p} vs. {q})")
    if products:
        raise TestFailure(f"{len(products)} fewer wakeups than expected")


def check_fifo(body: list[str]) -> None:
    thread_cnt, iter_cnt = 16, 16
    iterations = [l for l in body if "iteration:" in l]
    if not iterations:
        raise TestFailure("no iterations found")
    numbering = [int(n) for n in iterations[0].split() if n.isdigit()]
    if len(numbering) != thread_cnt:
        raise TestFailure(f"first iteration lists {len(numbering)} threads")
    if sorted(numbering) != list(range(thread_cnt)):
        raise TestFailure("first iteration does not list all threads")
    for i, it in enumerate(iterations[1:], start=1):
        if it != iterations[0]:
            raise TestFailure(f"iteration {i} differs from iteration 0")
    if len(iterations) != iter_cnt:
        raise TestFailure(f"{len(iterations)} iterations, {iter_cnt} expected")


def check_one(test: str, suite: str = "threads") -> None:
    if suite == "userprog":
        out = run_qemu_userprog(test)
        common_checks(out)
        body = extract_body(out, test)
        check_expected(test, body)
        return
    out = run_qemu(test)
    common_checks(out)
    body = extract_body(out, test)
    if test in ALARM_TESTS:
        check_alarm(test, ALARM_TESTS[test], body)
    elif test == FIFO_TEST:
        check_fifo(body)
    else:
        check_expected(test, body)


def main() -> None:
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    suite = "threads"
    if args[0] in ("--threads", "--userprog"):
        suite = args[0][2:]
        args = args[1:]
    global CK_DIR
    CK_DIR = os.path.join(KAIST, "tests", suite)
    if not args:
        sys.exit("no tests given")
    if args[0] == "--all":
        args = sorted(f[:-3] for f in os.listdir(CK_DIR) if f.endswith(".ck"))
        if suite == "threads":
            # mlfqs suite is a separate scheduler; not yet ported.
            args = [t for t in args if not t.startswith("mlfqs-")]
        else:
            # 'null' has no program; 'persistence' needs prior state.
            args = [t for t in args if t != "null" and "persistence" not in t]
        known = set(USERPROG_ARGS) | {f[:-3] for f in os.listdir(UPROG_DIR)}
        args = [t for t in args if t in known]

    failures = 0
    for test in args:
        try:
            check_one(test, suite)
            print(f"PASS {test}")
        except TestFailure as e:
            failures += 1
            print(f"FAIL {test}: {e}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
