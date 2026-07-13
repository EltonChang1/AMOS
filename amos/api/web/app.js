const state = {
  artifactId: null,
  artifact: null,
  identity: localStorage.getItem("amos.identity") || "analyst_001",
  loadingTimer: null,
  toastTimer: null,
};

const elements = {
  form: document.querySelector("#analysis-form"),
  question: document.querySelector("#question-input"),
  runButton: document.querySelector("#run-button"),
  identity: document.querySelector("#identity-select"),
  loadingPanel: document.querySelector("#loading-panel"),
  loadingTitle: document.querySelector("#loading-title"),
  loadingDetail: document.querySelector("#loading-detail"),
  progressBar: document.querySelector("#progress-bar"),
  resultSection: document.querySelector("#result-section"),
  valueGrid: document.querySelector("#value-grid"),
  report: document.querySelector("#report-content"),
  chartFigure: document.querySelector("#chart-figure"),
  chart: document.querySelector("#result-chart"),
  claims: document.querySelector("#claim-list"),
  supportDetail: document.querySelector("#support-detail"),
  history: document.querySelector("#history-list"),
  historyCount: document.querySelector("#history-count"),
  verificationLabel: document.querySelector("#verification-label"),
  coverageValue: document.querySelector("#coverage-value"),
  coverageRing: document.querySelector("#coverage-ring"),
  claimCount: document.querySelector("#claim-count"),
  memoryCount: document.querySelector("#memory-count"),
  replayButton: document.querySelector("#replay-button"),
  reviewButton: document.querySelector("#review-button"),
  reviewDialog: document.querySelector("#review-dialog"),
  reviewForm: document.querySelector("#review-form"),
  reviewInput: document.querySelector("#review-input"),
  reviewMessage: document.querySelector("#review-message"),
  toast: document.querySelector("#toast"),
};

const loadingSteps = [
  ["Resolving the approved metric definition…", "Checking accessible memory and the active stream watermark.", 18],
  ["Validating the query plan…", "Applying schema, permission, and event-time checks before execution.", 42],
  ["Running the bounded analysis…", "Computing the baseline, current window, and contributor concentration.", 67],
  ["Attaching evidence to every claim…", "Recording query hashes, data state, and replay instructions.", 86],
  ["Packaging the verified answer…", "Finalizing the report and evidence ledger.", 96],
];

function identityHeaders() {
  return { "Content-Type": "application/json", "X-AMOS-User": state.identity };
}

async function api(path, options = {}) {
  const response = await fetch(path, {
    ...options,
    headers: { ...identityHeaders(), ...(options.headers || {}) },
  });
  let body;
  try {
    body = await response.json();
  } catch {
    body = {};
  }
  if (!response.ok) {
    throw new Error(body.detail || `Request failed (${response.status})`);
  }
  return body;
}

function escapeHtml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function formatInline(value) {
  return escapeHtml(value).replace(/`([^`]+)`/g, "<code>$1</code>");
}

function renderMarkdown(markdown) {
  const lines = markdown.split("\n");
  const output = [];
  let listOpen = false;

  const closeList = () => {
    if (listOpen) output.push("</ul>");
    listOpen = false;
  };

  for (const rawLine of lines) {
    const line = rawLine.trim();
    if (!line || line.startsWith("![")) {
      closeList();
      continue;
    }
    if (line.startsWith("## ")) {
      closeList();
      const title = line.slice(3);
      if (["Chart", "Provenance", "Verification Status"].includes(title)) continue;
      output.push(`<h2>${formatInline(title)}</h2>`);
      continue;
    }
    if (line.startsWith("# ")) {
      closeList();
      output.push(`<h1>${formatInline(line.slice(2))}</h1>`);
      continue;
    }
    if (line.startsWith("- ")) {
      if (!listOpen) output.push("<ul>");
      listOpen = true;
      output.push(`<li>${formatInline(line.slice(2))}</li>`);
      continue;
    }
    if (line === "Warnings:" || line === "pass" || line === "warning" || line === "fail") continue;
    closeList();
    output.push(`<p>${formatInline(line)}</p>`);
  }
  closeList();
  return output.join("");
}

function formatDate(value) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "Recent";
  return new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(date);
}

function startLoading() {
  let index = 0;
  elements.loadingPanel.hidden = false;
  elements.resultSection.hidden = true;
  elements.valueGrid.hidden = true;
  elements.runButton.disabled = true;
  const applyStep = () => {
    const [title, detail, progress] = loadingSteps[index];
    elements.loadingTitle.textContent = title;
    elements.loadingDetail.textContent = detail;
    elements.progressBar.style.width = `${progress}%`;
    index = Math.min(index + 1, loadingSteps.length - 1);
  };
  applyStep();
  state.loadingTimer = window.setInterval(applyStep, 1250);
}

function stopLoading() {
  window.clearInterval(state.loadingTimer);
  state.loadingTimer = null;
  elements.loadingPanel.hidden = true;
  elements.runButton.disabled = false;
  elements.progressBar.style.width = "14%";
}

async function runAnalysis(event) {
  event.preventDefault();
  const question = elements.question.value.trim();
  if (!question) return;
  startLoading();
  try {
    const result = await api("/tasks/run", {
      method: "POST",
      body: JSON.stringify({ user_request: question, provenance_level: 3 }),
    });
    if (!result.artifact_id) throw new Error((result.errors || result.warnings || ["Analysis could not be completed."])[0]);
    await loadArtifact(result.artifact_id, true);
    await loadHistory();
  } catch (error) {
    stopLoading();
    elements.valueGrid.hidden = false;
    showToast(`${error.message}. If this is a fresh checkout, seed the demo data first.`, true);
  }
}

async function loadArtifact(artifactId, shouldScroll = false) {
  const detail = await api(`/artifacts/${encodeURIComponent(artifactId)}`);
  if (!detail.artifact) throw new Error(detail.warnings?.[0] || "Analysis is unavailable.");
  state.artifactId = artifactId;
  state.artifact = detail;
  stopLoading();
  renderArtifact(detail);
  if (shouldScroll) elements.resultSection.scrollIntoView({ behavior: "smooth", block: "start" });
}

function renderArtifact(detail) {
  const coverage = Math.round(detail.provenance_coverage * 100);
  elements.valueGrid.hidden = true;
  elements.resultSection.hidden = false;
  elements.report.innerHTML = renderMarkdown(detail.report_markdown);
  elements.coverageValue.textContent = `${coverage}%`;
  elements.coverageRing.textContent = coverage;
  elements.claimCount.textContent = detail.claims.length;
  const supportIds = new Set(detail.citations.flatMap((citation) => citation.memory_object_ids || []));
  elements.memoryCount.textContent = supportIds.size;
  elements.verificationLabel.textContent = detail.status === "pass" ? "All checks passed" : "Passed with warnings";
  document.querySelector("#result-time").textContent = `${formatDate(detail.artifact.created_at)} · ${detail.artifact.created_by}`;

  if (detail.chart_urls?.length) {
    elements.chart.src = detail.chart_urls[0];
    elements.chartFigure.hidden = false;
  } else {
    elements.chartFigure.hidden = true;
  }

  elements.claims.innerHTML = detail.claims.map((claim, index) => `
    <button class="claim-button" type="button" data-claim-id="${escapeHtml(claim.claim_id)}">
      <span class="claim-icon">✓</span>
      <span><strong>${escapeHtml(claim.claim_text)}</strong><small>${escapeHtml(claim.claim_type)} claim</small></span>
      <span class="claim-chevron">›</span>
    </button>
  `).join("");
  elements.supportDetail.hidden = true;
  elements.claims.querySelectorAll(".claim-button").forEach((button) => {
    button.addEventListener("click", () => showClaimSupport(button.dataset.claimId, button));
  });
  updateActiveHistory();
}

function showClaimSupport(claimId, button) {
  const citation = state.artifact.citations.find((item) => item.claim_id === claimId);
  document.querySelectorAll(".claim-button").forEach((item) => item.classList.toggle("active", item === button));
  if (!citation) {
    elements.supportDetail.innerHTML = "<p>No support record is available for this claim.</p>";
  } else {
    const supports = citation.support || [];
    elements.supportDetail.innerHTML = `
      <h4>Recorded support</h4>
      <p>${escapeHtml(citation.claim_text || "This claim is linked to the evidence below.")}</p>
      <div class="support-chips">${supports.map((item) => `<span title="${escapeHtml(item)}">${escapeHtml(item)}</span>`).join("")}</div>
    `;
  }
  elements.supportDetail.hidden = false;
}

async function loadHistory() {
  try {
    const result = await api("/artifacts?limit=12");
    const artifacts = result.artifacts || [];
    elements.historyCount.textContent = artifacts.length;
    if (!artifacts.length) {
      elements.history.innerHTML = '<div class="history-empty">Your verified runs will appear here.</div>';
      return;
    }
    elements.history.innerHTML = artifacts.map((artifact) => `
      <button class="history-item" type="button" data-artifact-id="${escapeHtml(artifact.artifact_id)}">
        <strong>${escapeHtml(artifact.user_request)}</strong>
        <span>${formatDate(artifact.created_at)} · ${escapeHtml(artifact.created_by)}</span>
      </button>
    `).join("");
    elements.history.querySelectorAll(".history-item").forEach((button) => {
      button.addEventListener("click", async () => {
        try {
          await loadArtifact(button.dataset.artifactId, true);
        } catch (error) {
          showToast(error.message, true);
        }
      });
    });
    updateActiveHistory();
  } catch (error) {
    elements.history.innerHTML = `<div class="history-empty">${escapeHtml(error.message)}</div>`;
  }
}

function updateActiveHistory() {
  elements.history.querySelectorAll(".history-item").forEach((button) => {
    button.classList.toggle("active", button.dataset.artifactId === state.artifactId);
  });
}

async function replayArtifact() {
  if (!state.artifactId) return;
  const original = elements.replayButton.innerHTML;
  elements.replayButton.disabled = true;
  elements.replayButton.textContent = "Replaying…";
  try {
    const result = await api(`/artifacts/${encodeURIComponent(state.artifactId)}/replay`, { method: "POST" });
    if (result.replay_status === "pass") showToast("Replay passed — query result hashes match the recorded analysis.");
    else showToast((result.errors || result.warnings || ["Replay needs review."])[0], true);
  } catch (error) {
    showToast(error.message, true);
  } finally {
    elements.replayButton.disabled = false;
    elements.replayButton.innerHTML = original;
  }
}

async function saveReview(event) {
  event.preventDefault();
  if (!state.artifactId) return;
  const feedback = elements.reviewInput.value.trim();
  if (!feedback) return;
  const reviewerApproved = state.identity === "reviewer_001";
  elements.reviewMessage.textContent = "Saving governed memory…";
  try {
    await api("/memory/feedback", {
      method: "POST",
      body: JSON.stringify({
        artifact_id: state.artifactId,
        reviewer_role: reviewerApproved ? "analytics_reviewer" : "analyst",
        feedback,
        authority: reviewerApproved ? "reviewer_approved" : "user_note",
      }),
    });
    elements.reviewDialog.close();
    elements.reviewInput.value = "";
    elements.reviewMessage.textContent = "";
    showToast(reviewerApproved ? "Approved review saved. It can inform the next run." : "Analyst note saved as governed, unapproved memory.");
  } catch (error) {
    elements.reviewMessage.textContent = error.message;
  }
}

function showToast(message, isError = false) {
  window.clearTimeout(state.toastTimer);
  elements.toast.textContent = message;
  elements.toast.style.background = isError ? "#6e302d" : "#17231c";
  elements.toast.hidden = false;
  state.toastTimer = window.setTimeout(() => { elements.toast.hidden = true; }, 5200);
}

elements.form.addEventListener("submit", runAnalysis);
elements.replayButton.addEventListener("click", replayArtifact);
elements.reviewButton.addEventListener("click", () => elements.reviewDialog.showModal());
elements.reviewForm.addEventListener("submit", saveReview);
document.querySelector("#cancel-review").addEventListener("click", () => elements.reviewDialog.close());
document.querySelector("#close-review").addEventListener("click", () => elements.reviewDialog.close());
document.querySelector("#new-analysis-button").addEventListener("click", () => {
  elements.resultSection.hidden = true;
  elements.valueGrid.hidden = false;
  elements.question.focus();
  window.scrollTo({ top: 0, behavior: "smooth" });
});
document.querySelectorAll(".suggestion").forEach((button) => {
  button.addEventListener("click", () => {
    elements.question.value = button.dataset.question;
    elements.question.focus();
  });
});

elements.identity.value = state.identity;
elements.identity.addEventListener("change", async () => {
  state.identity = elements.identity.value;
  localStorage.setItem("amos.identity", state.identity);
  state.artifactId = null;
  elements.resultSection.hidden = true;
  elements.valueGrid.hidden = false;
  await loadHistory();
  showToast(`Switched to ${elements.identity.options[elements.identity.selectedIndex].text}.`);
});

loadHistory();
