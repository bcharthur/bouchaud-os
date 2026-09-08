#!/usr/bin/env bash
#
# Pose la protection de `main` : les trois verdicts de barriere deviennent
# obligatoires, et l'historique cesse d'etre reecrivable.
#
# POURQUOI CE FICHIER EXISTE A COTE DU .ps1
#
# `configure_protection.ps1` demande PowerShell et `gh`. Ce script-ci ne
# demande que `curl` et un jeton. La protection de `main` est la seule chose du
# projet qui rend executoires les soixante-sept garde-fous : elle ne doit pas
# dependre du systeme d'exploitation de qui la pose.
#
# CE QUE LA PROTECTION FAIT, ET CE QU'ELLE NE FAIT PAS
#
# Elle exige `fast-gate`, `integration-gate` et `reliability-gate` -- les trois
# verdicts UNIQUES que les workflows produisent exprès pour elle, avec
# `if: always()`, afin qu'ils rapportent meme quand tout le reste est saute.
# Exiger les jobs individuels bloquerait pour toujours une PR qui n'en
# declenche qu'une partie.
#
# Elle interdit le force-push et la suppression : ce sont les deux seules
# operations qu'on ne peut pas defaire.
#
# `enforce_admins` reste FAUX. Ce n'est pas un relachement : c'est la porte de
# sortie d'urgence. Une protection qui enferme aussi celui qui l'a posee finit
# par etre retiree en entier le jour ou il faut passer, et on perd tout au lieu
# de passer une fois.
set -uo pipefail

DEPOT="${1:-bcharthur/bouchaud-os}"
JETON="${GH_TOKEN:-${GITHUB_TOKEN:-}}"

if [ -z "$JETON" ]; then
    if command -v gh >/dev/null 2>&1; then
        JETON="$(gh auth token 2>/dev/null || true)"
    fi
fi
if [ -z "$JETON" ]; then
    echo "ERREUR: aucun jeton. Exportez GH_TOKEN, ou authentifiez-vous avec gh." >&2
    echo "        Le jeton doit porter la portee 'administration:write' sur le depot." >&2
    exit 2
fi

CHARGE="$(cat <<'JSON'
{
  "required_status_checks": {
    "strict": true,
    "contexts": ["fast-gate", "integration-gate", "reliability-gate"]
  },
  "enforce_admins": false,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": false,
    "require_code_owner_reviews": false,
    "required_approving_review_count": 0,
    "require_last_push_approval": false
  },
  "restrictions": null,
  "required_linear_history": false,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "block_creations": false,
  "required_conversation_resolution": true,
  "lock_branch": false,
  "allow_fork_syncing": true
}
JSON
)"

echo "Protection de main sur $DEPOT"
REPONSE="$(mktemp)"
CODE="$(curl -sS -o "$REPONSE" -w '%{http_code}' -X PUT \
    -H "Authorization: Bearer $JETON" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    --data "$CHARGE" \
    "https://api.github.com/repos/$DEPOT/branches/main/protection")"

if [ "$CODE" != "200" ]; then
    echo "ERREUR: l'API a rendu $CODE" >&2
    head -c 800 "$REPONSE" >&2
    echo >&2
    rm -f "$REPONSE"
    exit 1
fi
rm -f "$REPONSE"

echo "BOUCHAUD_PROTECTION_MAIN_OK checks=fast-gate,integration-gate,reliability-gate"
echo
echo "Verification :"
curl -sS \
    -H "Authorization: Bearer $JETON" \
    -H "Accept: application/vnd.github+json" \
    "https://api.github.com/repos/$DEPOT/branches/main" \
    | grep -o '"protected":[a-z]*'
