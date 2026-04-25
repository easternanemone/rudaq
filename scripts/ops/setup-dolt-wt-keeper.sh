#!/usr/bin/env bash
# Sets up a periodic "working tree reset" daemon on the dolt server host.
#
# Why this exists
# ===============
# The shared dolt sql-server (on ai-proxy) holds the rust_daq database
# CHECKED OUT to serve queries. When `bd dolt push` updates the branch on
# the server, the server's working tree is left showing the diff between
# its (now stale) checkout and the new HEAD. That stale-WIP state blocks
# the NEXT push with "Error 1105: target has uncommitted changes".
#
# The fix is to periodically `dolt_reset --hard` on the server. The reset
# is safe because the working tree is purely a side-effect of the server
# holding the DB checked out — no real data lives there.
#
# Run this script ONCE on the dolt server host (ai-proxy).
set -euo pipefail

SERVER_DIR="${HOME}/.beads/shared-server"
mkdir -p "${SERVER_DIR}"

# Make sure pymysql is installed for the reset script
pip3 install --user --break-system-packages pymysql >/dev/null 2>&1 || true

cat > "${SERVER_DIR}/reset-rust-daq-wt-burst.sh" << 'INNER'
#!/usr/bin/env bash
# Cron-fired every minute; loops 5 times (10s apart) to give an effective
# polling cadence of ~10s. Resets rust_daq working tree to HEAD if dirty.
set -u
exec >> ~/.beads/shared-server/reset-wt.log 2>&1
for i in 1 2 3 4 5; do
  python3 - <<PY 2>/dev/null
import pymysql
try:
    conn = pymysql.connect(host="127.0.0.1", port=3308, user="root",
                            database="rust_daq", autocommit=True, connect_timeout=2)
    cur = conn.cursor()
    cur.execute("SELECT COUNT(*) FROM dolt_status")
    if cur.fetchone()[0] > 0:
        cur.execute("CALL DOLT_RESET('--hard')")
    conn.close()
except Exception:
    pass
PY
  [ $i -lt 5 ] && sleep 10
done
INNER

chmod +x "${SERVER_DIR}/reset-rust-daq-wt-burst.sh"

# Install crontab entry (idempotent)
CRON_LINE="* * * * * ${SERVER_DIR}/reset-rust-daq-wt-burst.sh"
(crontab -l 2>/dev/null | grep -v "reset-rust-daq" ; echo "${CRON_LINE}") | crontab -

echo "Installed:"
echo "  Script: ${SERVER_DIR}/reset-rust-daq-wt-burst.sh"
echo "  Log:    ${SERVER_DIR}/reset-wt.log"
echo "  Cron:   ${CRON_LINE}"
