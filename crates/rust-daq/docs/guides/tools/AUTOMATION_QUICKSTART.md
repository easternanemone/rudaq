# Automated Jules System - Quick Start

## What Is This?

An automated system that runs every 30 minutes to:
1. Review PRs with Gemini CLI
2. Auto-merge approved PRs (score ≥8)
3. Deploy new Jules agents when slots available

## Setup (3 Steps)

### 1. Test It
```bash
cd /Users/briansquires/code/rust-daq
./scripts/test_automation.sh
```

### 2. Run Manually Once
```bash
./scripts/auto_review_and_deploy.sh
```

### 3. Install Cron Job
```bash
./scripts/setup_cron.sh
```

Done! Now it runs automatically every 30 minutes.

## Check Status

```bash
# Quick status
./scripts/monitor_jules.sh

# View today's log
tail -f logs/auto_review_$(date +%Y%m%d).log

# Check state
cat .jules_automation_state.json | jq .
```

## What It Does Automatically

**Every 30 minutes:**
- ✓ Checks Jules session status
- ✓ Reviews new PRs with Gemini
- ✓ Merges PRs with score ≥8
- ✓ Deploys new agents when slots free

**Wave Deployment:**
- Wave 4: Code Quality (5 agents)
- Wave 5: Testing (6 agents)
- Wave 6: Infrastructure (4 agents)

## Safety

- Never reviews same PR twice
- Never deploys same wave twice
- Only merges with score ≥8 + APPROVE
- Respects 15 concurrent session limit
- Logs everything

## Stop It

```bash
# Remove cron job
crontab -l | grep -v 'auto_review_and_deploy.sh' | crontab -
```

## Restart It

```bash
# Add cron job back
./scripts/setup_cron.sh
```

## Files

- `scripts/auto_review_and_deploy.sh` - Main script
- `scripts/setup_cron.sh` - Install cron
- `scripts/test_automation.sh` - Test mode
- `.jules_automation_state.json` - State tracking
- `logs/auto_review_*.log` - Daily logs
- `AUTOMATION_GUIDE.md` - Full documentation

## Common Commands

```bash
# View cron jobs
crontab -l

# Manual run
./scripts/auto_review_and_deploy.sh

# Check logs
ls -lht logs/ | head -5

# View state
cat .jules_automation_state.json | jq .

# Monitor sessions
./scripts/monitor_jules.sh

# List PRs
gh pr list --repo TheFermiSea/rust-daq
```

## Troubleshooting

**Cron not running?**
```bash
tail -100 logs/cron.log
crontab -l
```

**No PRs being reviewed?**
```bash
gh pr list --repo TheFermiSea/rust-daq
cat logs/auto_review_*.log | grep "Reviewing PR"
```

**Agents not deploying?**
```bash
./scripts/monitor_jules.sh  # Check available slots
cat .jules_automation_state.json | jq .deployed_agents
```

## Expected Timeline

- **t=0**: Install cron job
- **t=30min**: First automated run
- **t=1-2hr**: Wave 4 deployed (if slots available)
- **t=2-4hr**: Waves 5-6 deployed
- **t=4-24hr**: PRs created, reviewed, merged
- **Ongoing**: Fully autonomous operation

## Full Documentation

For more detailed information on the automation system, refer to the relevant sections within the `docs/project_management/` directory.

---
*Note: "Jules" and "Gemini CLI" refer to internal AI agents used for development and code management within this project.*
