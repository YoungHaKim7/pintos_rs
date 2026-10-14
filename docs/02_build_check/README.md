```bash
just build 2>&1 | grep -E "kernel.bin|rror" | head -3; bash /tmp/bt.sh 2>&1 | tail -20)

cd build && timeout 25 python3 ../pintos-kaist/utils/pintos -- -q < /dev/null > /tmp/m1.txt 2>&1; tr -d '\r' < /tmp/m1.txt | grep -v "^$" | head -30
```
