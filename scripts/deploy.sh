#!/usr/bin/env bash
# Deploy Lush AI Music Studio from the dev machine (Git Bash on Windows), from the committed HEAD.
#   bash scripts/deploy.sh [--target auto|vps|render|all|none] [--dry-run] [--skip-tests]
#
# auto (default): deploy only the parts whose files changed since the revision running on each server.
#   VPS (control plane): apps/api, apps/web, infrastructure/vps  -> Docker build, /opt/lush/deploy.sh (auto-rollback)
#   Render node (worker): workers/, infrastructure/render-node    -> install.sh, restart lush-worker (rollback on failure)
# The running revision is stored in /opt/lush/current/REVISION and /mnt/ai-studio/AiStudio/lush-worker/REVISION.
# Docs: docs/DEPLOY.md. Server details: docs/VPS_DEPLOYMENT.md, docs/RENDER_NODE_DEPLOYMENT.md.
set -euo pipefail

VPS_HOST="${VPS_HOST:-lush1-vps}"
RENDER_HOST="${RENDER_HOST:-render}"
PUBLIC_URL="${PUBLIC_URL:-https://suno.darkambient.co}"
LUSH_ROOT="${LUSH_ROOT:-/mnt/ai-studio/AiStudio}"
KEEP_RELEASES=5
VPS_PATHS=(apps/api apps/web infrastructure/vps)
RENDER_PATHS=(workers infrastructure/render-node)

TARGET=auto DRY_RUN=0 SKIP_TESTS=0
while [ $# -gt 0 ]; do
	case "$1" in
		--target) TARGET="${2:?--target needs a value}"; shift 2 ;;
		--dry-run) DRY_RUN=1; shift ;;
		--skip-tests) SKIP_TESTS=1; shift ;;
		*) echo "unknown argument: $1" >&2; exit 2 ;;
	esac
done
case "$TARGET" in auto|vps|render|all|none) ;; *) echo "bad --target: $TARGET" >&2; exit 2 ;; esac

REPO="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
SSH=(ssh -o BatchMode=yes -o ConnectTimeout=15)

step() { printf '\n==> %s\n' "$*"; }
die() { printf '\nDEPLOY FAILED: %s\n' "$*" >&2; exit 1; }

# ---------- 1. Local preflight ----------
step "Preflight"
[ -z "$(git status --porcelain)" ] || { git status --short; die "working tree is not clean: commit or stash first"; }
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[ "$BRANCH" = main ] || die "on branch '$BRANCH'; deploy only from main"
HEAD_REV="$(git rev-parse HEAD)"
SHORT="$(git rev-parse --short HEAD)"
if git remote get-url origin >/dev/null 2>&1; then
	git fetch -q origin main || die "git fetch origin failed"
	[ "$(git rev-parse origin/main)" = "$HEAD_REV" ] || die "HEAD $SHORT is not what origin/main has: push first (git push origin main)"
	echo "HEAD $SHORT = origin/main"
else
	echo "WARNING: no git remote 'origin'; deploying an unpushed commit $SHORT"
fi

# ---------- 2. What runs on the servers ----------
step "Reading deployed revisions"
"${SSH[@]}" "$VPS_HOST" true || die "cannot ssh to $VPS_HOST"
"${SSH[@]}" "$RENDER_HOST" true || die "cannot ssh to $RENDER_HOST"
VPS_REV="$("${SSH[@]}" "$VPS_HOST" 'cat /opt/lush/current/REVISION 2>/dev/null' || true)"
RENDER_REV="$("${SSH[@]}" "$RENDER_HOST" "cat $LUSH_ROOT/lush-worker/REVISION 2>/dev/null" || true)"
echo "VPS:    ${VPS_REV:-unknown}"
echo "Render: ${RENDER_REV:-unknown}"
echo "HEAD:   $HEAD_REV"

# needs_deploy <deployed-rev> <paths...>: 0 when the target must be deployed.
needs_deploy() {
	local rev="$1"; shift
	[ -n "$rev" ] && git cat-file -e "$rev^{commit}" 2>/dev/null || return 0
	[ -n "$(git diff --name-only "$rev" HEAD -- "$@")" ]
}

DO_VPS=0 DO_RENDER=0
case "$TARGET" in
	vps) DO_VPS=1 ;;
	render) DO_RENDER=1 ;;
	all) DO_VPS=1; DO_RENDER=1 ;;
	auto)
		needs_deploy "$VPS_REV" "${VPS_PATHS[@]}" && DO_VPS=1
		needs_deploy "$RENDER_REV" "${RENDER_PATHS[@]}" && DO_RENDER=1
		;;
esac

# ---------- 3. Config drift: root-owned files the script never overwrites ----------
if [ "$DO_VPS" = 1 ]; then
	step "Checking VPS config drift"
	"${SSH[@]}" "$VPS_HOST" 'cat /etc/systemd/system/lush-api.service' | diff -u - infrastructure/vps/lush-api.service \
		|| die "lush-api.service on the VPS differs from the repo: install it by hand (systemctl daemon-reload) after review"
	"${SSH[@]}" "$VPS_HOST" 'cat /etc/nginx/sites-available/lush-studio.conf' | diff -u - infrastructure/vps/nginx-lush-studio.conf \
		|| die "nginx config on the VPS differs from the repo: shared nginx, apply by hand (nginx -t && systemctl reload nginx) after review"
	echo "no drift"
fi

step "Plan"
echo "VPS:    $([ $DO_VPS = 1 ] && echo DEPLOY || echo skip)"
echo "Render: $([ $DO_RENDER = 1 ] && echo DEPLOY || echo skip)"
if [ "$DO_VPS" = 1 ] && [ -n "$VPS_REV" ] && git cat-file -e "$VPS_REV^{commit}" 2>/dev/null; then
	MIGRATIONS="$(git diff --name-only --diff-filter=A "$VPS_REV" HEAD -- apps/api/migrations)"
	[ -z "$MIGRATIONS" ] || printf 'NEW MIGRATIONS (rollback then needs a DB restore from /var/backups/lush):\n%s\n' "$MIGRATIONS"
fi
[ "$DRY_RUN" = 1 ] && { echo "dry run: stopping here"; exit 0; }
[ "$DO_VPS" = 0 ] && [ "$DO_RENDER" = 0 ] && { echo "nothing to deploy"; exit 0; }

# ---------- 4. Tests and build (on the dev machine) ----------
if [ "$DO_VPS" = 1 ]; then
	if [ "$SKIP_TESTS" = 0 ]; then
		step "API tests (cargo test)"
		if tasklist //FI "IMAGENAME eq lush-api.exe" 2>/dev/null | grep -qi lush-api.exe; then
			die "lush-api.exe is running locally: stop it, cargo test fails with 'Access is denied'"
		fi
		(cd apps/api && cargo test --locked -q) || die "cargo test failed"
	fi
	step "Web tests and build"
	(
		cd apps/web
		if [ ! -d node_modules ] || [ package-lock.json -nt node_modules/.package-lock.json ]; then npm ci --no-audit --no-fund; fi
		[ "$SKIP_TESTS" = 1 ] || npm test
		npm run build
	) || die "web test/build failed"
	tar -czf "$TMP/web-dist.tgz" -C apps/web --transform 's,^dist,web,' dist
fi
if [ "$DO_RENDER" = 1 ] && [ "$SKIP_TESTS" = 0 ]; then
	step "Worker tests (pytest)"
	(
		cd workers/music-worker
		[ -x .venv/Scripts/python.exe ] || py -m venv .venv
		.venv/Scripts/python.exe -m pip install -q -e ".[dev]"
		.venv/Scripts/python.exe -m pytest -q
	) || die "worker tests failed"
fi

# ---------- 5. VPS ----------
if [ "$DO_VPS" = 1 ]; then
	step "VPS: upload source $SHORT"
	git archive --format=tar HEAD | "${SSH[@]}" "$VPS_HOST" 'rm -rf /opt/lush-src && mkdir -p /opt/lush-src && tar -x -C /opt/lush-src'
	scp -q -o BatchMode=yes "$TMP/web-dist.tgz" "$VPS_HOST:/opt/lush-src/web-dist.tgz"

	step "VPS: build in Docker and install (deploy.sh backs up the DB and rolls back on a failed health check)"
	"${SSH[@]}" "$VPS_HOST" bash -s -- "$HEAD_REV" "$KEEP_RELEASES" <<'REMOTE' || die "VPS deploy failed (see output above; deploy.sh rolls back automatically)"
set -euo pipefail
REV="$1" KEEP="$2"
cd /opt/lush-src/apps/api
docker run --rm -v "$PWD":/src -v lush-cargo-registry:/usr/local/cargo/registry \
	-v lush-cargo-target:/src/target -w /src rust:1-bookworm \
	sh -c 'cargo build --release --locked && cp target/release/lush-api /src/lush-api.bin'
REL="$(mktemp -d)"
cp lush-api.bin "$REL/lush-api"
tar -xzf /opt/lush-src/web-dist.tgz -C "$REL"
echo "$REV" > "$REL/REVISION"
tar -czf /opt/lush-src/release.tar.gz -C "$REL" .
rm -rf "$REL"
# deploy.sh and backup.sh are ours: keep them in sync with the repo.
install -m 755 /opt/lush-src/infrastructure/vps/deploy.sh /opt/lush/deploy.sh
install -m 755 /opt/lush-src/infrastructure/vps/backup.sh /opt/lush/backup.sh
/opt/lush/deploy.sh /opt/lush-src/release.tar.gz
# Keep the newest releases; never remove the one in use.
CURRENT="$(readlink -f /opt/lush/current)"
ls -1d /opt/lush/releases/*/ | sed 's,/$,,' | sort -r | tail -n +"$((KEEP + 1))" | while read -r old; do
	[ "$old" = "$CURRENT" ] || rm -rf "$old"
done
REMOTE

	step "VPS: verify"
	[ "$("${SSH[@]}" "$VPS_HOST" 'cat /opt/lush/current/REVISION')" = "$HEAD_REV" ] || die "VPS is not running $SHORT (rolled back?)"
	HEALTH="$(curl -fsS --max-time 15 "$PUBLIC_URL/api/health")" || die "public health check failed: $PUBLIC_URL/api/health"
	echo "$HEALTH" | grep -q '"status":"ok"' || die "unexpected health response: $HEALTH"
	echo "VPS OK: $HEALTH"
fi

# ---------- 6. Render node ----------
# render_install <rev>: install that revision of the worker and restart it.
render_install() {
	local rev="$1" restart_acestep="$2"
	"${SSH[@]}" "$RENDER_HOST" "rm -rf $LUSH_ROOT/src && mkdir -p $LUSH_ROOT/src"
	git archive --format=tar "$rev" | "${SSH[@]}" "$RENDER_HOST" "tar -x -C $LUSH_ROOT/src"
	"${SSH[@]}" "$RENDER_HOST" bash -s -- "$LUSH_ROOT" "$rev" "$restart_acestep" <<'REMOTE'
set -euo pipefail
ROOT="$1" REV="$2" RESTART_ACESTEP="$3"
LUSH_ROOT="$ROOT" bash "$ROOT/src/infrastructure/render-node/install.sh" "$ROOT/src"
echo "$REV" > "$ROOT/lush-worker/REVISION"
if [ "$RESTART_ACESTEP" = 1 ]; then systemctl --user restart acestep-api; fi
date +%s > "$ROOT/lush-worker/.restarted_at"
# SIGTERM: the worker hands a running job back to the queue before exiting.
systemctl --user restart lush-worker
REMOTE
}

# render_healthy: the worker is active and registered with the control plane after the restart.
render_healthy() {
	"${SSH[@]}" "$RENDER_HOST" bash -s -- "$LUSH_ROOT" <<'REMOTE'
since="@$(cat "$1/lush-worker/.restarted_at")"
for _ in $(seq 1 30); do
	sleep 2
	systemctl --user is-active -q lush-worker || continue
	if journalctl --user -u lush-worker --since "$since" --no-pager -o cat | grep -q 'registered as node'; then
		journalctl --user -u lush-worker --since "$since" --no-pager -o cat | tail -3
		exit 0
	fi
done
journalctl --user -u lush-worker -n 30 --no-pager -o cat
exit 1
REMOTE
}

if [ "$DO_RENDER" = 1 ]; then
	RESTART_ACESTEP=0
	if needs_deploy "$RENDER_REV" infrastructure/render-node/acestep-api.service; then RESTART_ACESTEP=1; fi
	[ -z "$RENDER_REV" ] && RESTART_ACESTEP=0   # unknown history: do not reload the model needlessly
	step "Render: install worker $SHORT (restart acestep-api: $RESTART_ACESTEP)"
	render_install "$HEAD_REV" "$RESTART_ACESTEP" || die "render install failed (the previous worker may still be running; check journalctl --user -u lush-worker)"
	step "Render: verify"
	if ! render_healthy; then
		if [ -n "$RENDER_REV" ] && git cat-file -e "$RENDER_REV^{commit}" 2>/dev/null; then
			step "Render: worker unhealthy, rolling back to ${RENDER_REV:0:7}"
			render_install "$RENDER_REV" 0 && render_healthy && die "new worker failed; rolled back to ${RENDER_REV:0:7}"
			die "new worker failed AND rollback failed: fix by hand on $RENDER_HOST"
		fi
		die "new worker failed and no earlier revision is known to roll back to"
	fi
	echo "Render OK"
fi

step "DONE: deployed $SHORT ($([ $DO_VPS = 1 ] && echo 'VPS ')$([ $DO_RENDER = 1 ] && echo 'render'))"
