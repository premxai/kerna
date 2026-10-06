(() => {
  "use strict";

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const icon = (id) => `<svg aria-hidden="true"><use href="#${id}" /></svg>`;

  /* ---------- Theme ---------- */
  const root = document.documentElement;
  $("#theme-toggle")?.addEventListener("click", () => {
    const next = root.getAttribute("data-theme") === "dark" ? "light" : "dark";
    root.setAttribute("data-theme", next);
    try { localStorage.setItem("kerna-theme", next); } catch (e) { /* storage unavailable */ }
  });

  /* ---------- Header ---------- */
  const header = $("#header");
  const onScroll = () => header.classList.toggle("scrolled", window.scrollY > 8);
  onScroll();
  window.addEventListener("scroll", onScroll, { passive: true });

  const menuBtn = $("#menu-btn");
  const navLinks = $("#nav-links");
  menuBtn?.addEventListener("click", () => {
    const open = navLinks.classList.toggle("open");
    menuBtn.setAttribute("aria-expanded", String(open));
  });
  $$("a", navLinks).forEach((a) => a.addEventListener("click", () => {
    navLinks.classList.remove("open");
    menuBtn?.setAttribute("aria-expanded", "false");
  }));

  /* ---------- Copy buttons ---------- */
  $$(".copy").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const target = btn.dataset.copyTarget && $(btn.dataset.copyTarget);
      const text = (btn.dataset.copy || (target && target.innerText) || "").trim();
      if (!text) return;
      try {
        await navigator.clipboard.writeText(text);
        btn.dataset.copied = "true";
        btn.setAttribute("aria-label", "Copied");
        setTimeout(() => { delete btn.dataset.copied; btn.setAttribute("aria-label", "Copy"); }, 1600);
      } catch (e) {
        btn.setAttribute("aria-label", "Select the text to copy");
      }
    });
  });

  /* ---------- GitHub stars (hidden if unavailable) ---------- */
  (async () => {
    const wrap = $("[data-stars]");
    if (!wrap) return;
    try {
      const cached = JSON.parse(sessionStorage.getItem("kerna-stars") || "null");
      let count = cached && Date.now() - cached.t < 900000 ? cached.n : null;
      if (count === null) {
        const res = await fetch("https://api.github.com/repos/premxai/kerna");
        if (!res.ok) return;
        count = (await res.json()).stargazers_count;
        sessionStorage.setItem("kerna-stars", JSON.stringify({ n: count, t: Date.now() }));
      }
      if (typeof count !== "number") return;
      $("[data-star-count]", wrap).textContent = count.toLocaleString();
      wrap.hidden = false;
    } catch (e) { /* offline or rate-limited: leave hidden */ }
  })();

  /* ---------- Scroll reveal ---------- */
  const revealEls = $$(".reveal");
  if ("IntersectionObserver" in window && !reduceMotion) {
    const io = new IntersectionObserver((entries) => {
      entries.forEach((e) => { if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); } });
    }, { threshold: 0.12, rootMargin: "0px 0px -40px 0px" });
    revealEls.forEach((el) => io.observe(el));
  } else {
    revealEls.forEach((el) => el.classList.add("in"));
  }

  /* ---------- Tabs (generic) ---------- */
  function tabs(container, onSelect) {
    const list = $$("[role=tab]", container);
    const select = (tab) => {
      list.forEach((t) => t.setAttribute("aria-selected", String(t === tab)));
      onSelect(tab);
    };
    list.forEach((t, i) => {
      t.addEventListener("click", () => select(t));
      t.addEventListener("keydown", (e) => {
        if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
        const next = list[(i + (e.key === "ArrowRight" ? 1 : list.length - 1)) % list.length];
        next.focus();
        select(next);
      });
    });
  }

  const clientTabs = $("#client-tabs");
  clientTabs && tabs(clientTabs, (t) => {
    $$("[data-client-name]").forEach((el) => { el.textContent = t.dataset.client; });
  });

  const installTabs = $("#install-tabs");
  installTabs && tabs(installTabs, (t) => {
    $("#install-code").textContent = t.dataset.cmd;
    $("#install-label").textContent = t.dataset.label;
  });

  /* ---------- Approval demo ---------- */
  const approval = $("#approval");
  if (approval) {
    const chip = $("#approval-chip");
    const result = $("#approval-result");
    $$("[data-decide]", approval).forEach((btn) => btn.addEventListener("click", () => {
      const approved = btn.dataset.decide === "approved";
      chip.className = "chip " + (approved ? "allow" : "deny");
      chip.textContent = approved ? "Approved" : "Denied";
      result.textContent = approved
        ? "approval.decided → approved · tool starts"
        : "approval.decided → denied · tool never starts";
      $$("[data-decide]", approval).forEach((b) => { b.disabled = true; b.style.opacity = "0.5"; });
      setTimeout(() => {
        chip.className = "chip ask";
        chip.textContent = "Waiting";
        result.textContent = "Awaiting your decision…";
        $$("[data-decide]", approval).forEach((b) => { b.disabled = false; b.style.opacity = ""; });
      }, 2600);
    }));
  }

  /* ---------- Inspector ---------- */
  const CALLS = [
    { tool: "calendar.list_events", effect: "allow", rule: "allow-calendar-read", cap: "calendar.read", args: "today", calls: 1, secs: 2, digest: "a41f…9c2e", out: "3 events" },
    { tool: "weather.get_forecast", effect: "allow", rule: "allow-weather-read", cap: "weather.read", args: "home", calls: 2, secs: 4, digest: "07be…41d8", out: "forecast" },
    { tool: "notes.read_note", effect: "allow", rule: "allow-workspace-reads", cap: "file_read", args: "todo.md", calls: 3, secs: 6, digest: "c93a…e015", out: "1.2 KB" },
    { tool: "notes.write_note", effect: "ask", rule: "ask-before-writes", cap: "file_write", args: "brief.md", calls: 4, secs: 9, digest: "5d20…b7a3", out: "approved once", approved: true },
    { tool: "web.fetch_url", effect: "deny", rule: "deny-external-actions", cap: "network", args: "https://…", calls: 5, secs: 11, digest: "e8c4…2f90", out: "not started" },
  ];
  const LABEL = { allow: "Allowed", ask: "Asked", deny: "Denied" };

  const eventsEl = $("#events");
  const canvasEl = $("#canvas");
  const propsEl = $("#props");
  let current = 0;
  let timer = null;
  let interacted = false;

  if (eventsEl && canvasEl && propsEl) {
    eventsEl.innerHTML = CALLS.map((c, i) => `
      <li><button class="event" type="button" data-i="${i}" data-effect="${c.effect}" aria-pressed="false">
        <i></i><span>${c.tool}</span><em>${LABEL[c.effect].toLowerCase()}</em>
      </button></li>`).join("");

    const render = (i) => {
      current = i;
      const c = CALLS[i];
      const denied = c.effect === "deny";
      const policyState = c.effect === "ask" ? (c.approved ? "approved" : "waiting") : c.effect;
      const chipCls = denied ? "deny" : c.effect === "ask" ? "ask" : "allow";

      $$(".event", eventsEl).forEach((b) => b.setAttribute("aria-pressed", String(+b.dataset.i === i)));

      const node = (ico, title, sub, opts = {}) => `
        <div class="node${opts.off ? " off" : ""}${opts.selected ? " selected" : ""}">
          <span class="ico">${icon(ico)}</span>
          <div><h4>${title}</h4><p>${sub}</p></div>
          <span class="state">${opts.chip ? `<span class="chip ${opts.chip}">${opts.chipText}</span>` : ""}</span>
          ${opts.selected ? `<div class="selection" aria-hidden="true"><b></b><b></b><b></b><b></b><span class="tag">policy · ${c.effect}</span></div>` : ""}
        </div>`;
      const link = (off) => `<div class="link${off ? " off" : ""}"></div>`;

      canvasEl.innerHTML = `
        <div class="flow">
          ${node("i-model", "Model proposes", c.tool)}
          ${link(false)}
          ${node("i-shield", "Policy check", c.rule, { selected: true, chip: chipCls, chipText: policyState })}
          ${link(denied)}
          ${node("i-gauge", "Budget check", denied ? "skipped" : `tool calls ${c.calls}/20`, { off: denied, chip: "allow", chipText: "ok" })}
          ${link(denied)}
          ${node("i-box", "MCP tool", denied ? "never started" : "isolated child", { off: denied })}
          ${link(false)}
          ${node("i-receipt", "Receipt", c.digest, { chip: "plain", chipText: "recorded" })}
        </div>
        <span class="canvas-note">${denied ? "Denied calls never reach the tool." : "Every step is recorded."}</span>`;

      const pct = (n, d) => Math.round((n / d) * 100);
      propsEl.innerHTML = `
        <div class="panel-title"><span>Inspect</span></div>
        <div class="prop"><h5>Decision</h5>
          <div class="row"><span>Effect</span><span class="chip ${chipCls}">${LABEL[c.effect]}</span></div>
          <div class="row"><span>Rule</span><span>${c.rule}</span></div>
          ${c.effect === "ask" ? `<div class="row"><span>Approval</span><span>${c.approved ? "approved" : "pending"}</span></div>` : ""}
        </div>
        <div class="prop"><h5>Request</h5>
          <div class="row"><span>Tool</span><span>${c.tool}</span></div>
          <div class="row"><span>Capability</span><span>${c.cap}</span></div>
          <div class="row"><span>Arguments</span><span>redacted</span></div>
        </div>
        <div class="prop"><h5>Budget</h5>
          <div class="row"><span>Tool calls</span><span>${c.calls} / 20</span></div>
          <div class="meter"><i style="--v:${pct(c.calls, 20)}%"></i></div>
          <div class="row"><span>Runtime</span><span>${c.secs}s / 120s</span></div>
          <div class="meter"><i style="--v:${pct(c.secs, 120)}%"></i></div>
        </div>
        <div class="prop"><h5>Receipt</h5>
          <div class="row"><span>Digest</span><span>${c.digest}</span></div>
          <div class="row"><span>Result</span><span>${c.out}</span></div>
        </div>`;
    };

    const stop = () => { if (timer) { clearInterval(timer); timer = null; } };
    const markInteracted = () => {
      if (interacted) return;
      interacted = true;
      stop();
      canvasEl.setAttribute("aria-live", "polite");
      propsEl.setAttribute("aria-live", "polite");
    };
    eventsEl.addEventListener("click", (e) => {
      const b = e.target.closest(".event");
      if (!b) return;
      markInteracted();
      render(+b.dataset.i);
    });
    eventsEl.addEventListener("focusin", markInteracted);

    render(0);

    // Gentle autoplay while visible, until the visitor takes over.
    const inspector = $("#inspector");
    if (!reduceMotion && "IntersectionObserver" in window) {
      new IntersectionObserver((entries) => {
        entries.forEach((en) => {
          if (interacted) return;
          if (en.isIntersecting && !timer) timer = setInterval(() => render((current + 1) % CALLS.length), 3200);
          else if (!en.isIntersecting) stop();
        });
      }, { threshold: 0.4 }).observe(inspector);
      inspector.addEventListener("pointerenter", stop);
      inspector.addEventListener("pointerdown", markInteracted);
    }
  }
})();
