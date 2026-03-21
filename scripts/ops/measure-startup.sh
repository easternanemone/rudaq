#!/bin/bash
MODE=$1
echo "Measuring startup for mode: $MODE"
START_TIME=$(gdate +%s%3N || date +%s000)

timeout 5s ./target/debug/rust-daq-daemon daemon --runtime-mode $MODE > "startup.log" 2>&1 &
PID=$!

READY=0
for i in {1..50}; do
    if grep -q "DAQ gRPC server .* listening on" "startup.log"; then
        READY=1
        break
    fi
    sleep 0.1
done

END_TIME=$(gdate +%s%3N || date +%s000)

kill $PID || true
wait $PID 2>/dev/null || true

if [ "$READY" -eq 1 ]; then
    ELAPSED=$((END_TIME - START_TIME))
    echo "Mode $MODE: Ready in ${ELAPSED}ms"
else
    echo "Mode $MODE: Timed out waiting for ready"
    cat startup.log | head -n 20
fi

echo "--- Registrations ---"
grep "Registered" "startup.log"
echo "--- DB info ---"
grep "Database ready" "startup.log" || echo "No DB"
echo "============================="
