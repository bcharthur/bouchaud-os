$ErrorActionPreference = "Stop"

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "GitHub CLI (gh) est requis."
}

$repo = gh repo view --json nameWithOwner --jq '.nameWithOwner'
if (-not $repo) {
    throw "Impossible de determiner le depot GitHub."
}

$payload = @{
    required_status_checks = @{
        strict = $true
        contexts = @(
            "fast-gate",
            "integration-gate",
            "reliability-gate"
        )
    }
    enforce_admins = $false
    required_pull_request_reviews = @{
        dismiss_stale_reviews = $false
        require_code_owner_reviews = $false
        required_approving_review_count = 0
        require_last_push_approval = $false
    }
    restrictions = $null
    required_linear_history = $false
    allow_force_pushes = $false
    allow_deletions = $false
    block_creations = $false
    required_conversation_resolution = $true
    lock_branch = $false
    allow_fork_syncing = $true
} | ConvertTo-Json -Depth 8

Write-Host "Configuration de la protection de main sur $repo"
$payload | gh api `
    --method PUT `
    -H "Accept: application/vnd.github+json" `
    -H "X-GitHub-Api-Version: 2022-11-28" `
    "/repos/$repo/branches/main/protection" `
    --input -

Write-Host ""
Write-Host "Protection appliquee. Checks requis : fast-gate, integration-gate, reliability-gate"
