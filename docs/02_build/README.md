```bash
just build 2>&1 | grep -E "kernel.bin|rror" | head -3; bash /tmp/bt.sh 2>&1 | tail -20)
```
