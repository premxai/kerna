(() => {
  "use strict";

  /* ---------------- Navigation ---------------- */

  const menuButton = document.querySelector(".menu-toggle");
  const nav = document.querySelector(".site-nav");

  if (menuButton && nav) {
    menuButton.addEventListener("click", () => {
      const open = nav.classList.toggle("open");
      menuButton.setAttribute("aria-expanded", String(open));
    });
  }

  /* ---------------- GitHub stars ---------------- */

  const starNodes = document.querySelectorAll("[data-github-stars]");

  if (starNodes.length) {
    const cacheKey = "kerna-github-stars";
    const lifetime = 15 * 60 * 1000;
    const show = (count) => {
      const formatted = Number(count).toLocaleString();
      starNodes.forEach((node) => {
        node.textContent = formatted;
        node.closest(".github-link")?.setAttribute("aria-label", `View Kerna on GitHub. ${formatted} stars.`);
      });
    };
    try {
      const cached = JSON.parse(window.localStorage.getItem(cacheKey) || "null");
      if (cached && Date.now() - cached.savedAt < lifetime) show(cached.count);
    } catch {}
    fetch("https://api.github.com/repos/premxai/kerna", { headers: { Accept: "application/vnd.github+json" } })
      .then((r) => (r.ok ? r.json() : Promise.reject(r)))
      .then((repo) => {
        show(repo.stargazers_count);
        try {
          window.localStorage.setItem(cacheKey, JSON.stringify({ count: repo.stargazers_count, savedAt: Date.now() }));
        } catch {}
      })
      .catch(() => {});
  }

  /* ---------------- Post-story reveals ---------------- */

  const revealObserver = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          entry.target.classList.add("in");
          revealObserver.unobserve(entry.target);
        }
      }
    },
    { threshold: 0.15 }
  );
  document.querySelectorAll(".reveal").forEach((el) => revealObserver.observe(el));

  /* ---------------- Story engine ---------------- */

  const story = document.getElementById("story");
  const world = document.querySelector(".world");
  const stack = document.getElementById("stack");
  const packet = document.getElementById("packet");
  const signal = document.getElementById("signal");
  const plates = [...document.querySelectorAll(".plate")];
  const copyBlocks = [...document.querySelectorAll(".copy-block")];
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  if (!story || !world || reduced) return;

  const STORY_VH = 780;
  story.style.height = `${STORY_VH}vh`;

  const PHASES = {
    explode: [0.03, 0.14],
    inspect: [0.14, 0.26],
    route: [0.26, 0.42],
    govern: [0.42, 0.58],
    execute: [0.58, 0.73],
    observe: [0.73, 0.88],
    end: [0.88, 1.0],
  };
  const SCENES = [
    { name: "inspect", plate: 1 },
    { name: "route", plate: 2 },
    { name: "govern", plate: 3 },
    { name: "execute", plate: 4 },
    { name: "observe", plate: 5 },
  ];

  const clamp01 = (v) => Math.max(0, Math.min(1, v));
  const prog = (p, a, b) => clamp01((p - a) / (b - a));
  const smooth = (a, b, v) => {
    const t = clamp01((v - a) / (b - a));
    return t * t * (3 - 2 * t);
  };
  const easeOut = (t) => 1 - Math.pow(1 - t, 3);
  const lerp = (a, b, t) => a + (b - a) * t;
  const lerpPt = (A, B, t) => ({ x: lerp(A.x, B.x, t), y: lerp(A.y, B.y, t), z: lerp(A.z, B.z, t) });

  let W = 0, H = 0, FIT = 1, MOBILE = false;
  let exploded = [], startStack = [], plateH = [];

  function measure() {
    MOBILE = window.innerWidth < 960;
    W = world.clientWidth;
    H = world.clientHeight || window.innerHeight;
    FIT = Math.min(1, W / 620);
    const dx = MOBILE ? W * 0.16 : W * 0.15;
    const dy = MOBILE ? H * 0.14 : H * 0.13;
    exploded = [
      { x: MOBILE ? -W * 0.05 : -W * 0.3, y: -H * 0.3, z: 60, rx: 6, ry: -4 },
      { x: -dx, y: -dy * 1.4, z: -30, rx: 9, ry: -6 },
      { x: 0, y: 0, z: -120, rx: 12, ry: -8 },
      { x: dx, y: dy * 1.4, z: -210, rx: 15, ry: -10 },
      { x: MOBILE ? W * 0.05 : W * 0.3, y: dy * 2.8, z: -300, rx: 18, ry: -12 },
      { x: MOBILE ? W * 0.1 : W * 0.45, y: H * 0.3, z: -390, rx: 21, ry: -14 },
    ];
    startStack = exploded.map((pos, i) => ({ x: exploded[0].x, y: exploded[0].y, z: -20 - i * 14 }));
    plateH = plates.map((el) => el.offsetHeight);
  }

  const focusPos = () => ({ x: 0, y: MOBILE ? H * 0.16 : H * 0.02, z: 300 });

  function applyPlate(i, T, opacity) {
    const el = plates[i];
    el.style.opacity = String(clamp01(opacity));
    el.style.transform =
      `translate(-50%, -50%) translate3d(${T.x}px, ${T.y}px, ${T.z}px) ` +
      `rotateX(${T.rx || 0}deg) rotateY(${T.ry || 0}deg) scale(${FIT})`;
  }

  /* ---------------- Sub-animation elements ---------------- */

  const scanline = document.getElementById("scanline");
  const inspectRows = [...document.querySelectorAll(".inspect-row")];
  const junction = document.getElementById("junction");
  const junctionPacket = document.getElementById("junction-packet");
  const junctionShadow = document.getElementById("junction-shadow");
  const govRows = [...document.querySelectorAll(".gov-row")];
  const gate = document.getElementById("gate");
  const approvalCard = document.getElementById("approval-card");
  const approveBtn = document.getElementById("approve-btn");
  const govCleared = document.getElementById("gov-cleared");
  const chamber = document.getElementById("chamber");
  const runSteps = [...document.querySelectorAll(".run-step")];
  const tlRows = [...document.querySelectorAll(".tl-row")];
  const routePlate = document.getElementById("route-plate");
  const typedEl = document.getElementById("typed");

  /* ---------------- Route toggle interaction ---------------- */

  let lane = "cloud";
  let laneAnim = { from: 0, to: 0, start: 0 };
  document.querySelectorAll(".route-toggle button").forEach((btn) => {
    btn.addEventListener("click", () => {
      lane = btn.dataset.lane;
      document.querySelectorAll(".route-toggle button").forEach((b) => b.classList.toggle("on", b === btn));
      junction.querySelectorAll(".track").forEach((t) => t.classList.remove("active"));
      junction.querySelector(lane === "cloud" ? ".t-cloud" : ".t-local").classList.add("active");
      laneAnim = { from: null, to: null, start: performance.now() };
    });
  });

  const LANE_END = { cloud: { x: 240, y: 150 }, local: { x: 140, y: 140 } };
  const LANE_START = { x: 240, y: 22 };

  function junctionTick(now) {
    const activeScene = SCENES.find((s) => storyP >= PHASES[s.name][0] && storyP < PHASES[s.name][1]);
    let t;
    if (laneAnim.start) {
      t = clamp01((now - laneAnim.start) / 700);
    } else if (activeScene && activeScene.name === "route") {
      const s = prog(storyP, ...PHASES.route);
      t = smooth(0.25, 0.55, s);
    } else {
      t = storyP >= PHASES.route[1] ? 1 : 0;
    }
    const end = LANE_END[lane];
    junctionPacket.setAttribute("cx", String(lerp(LANE_START.x, end.x, t)));
    junctionPacket.setAttribute("cy", String(lerp(LANE_START.y, end.y, t)));

    const rs = activeScene && activeScene.name === "route" ? prog(storyP, ...PHASES.route) : (storyP > PHASES.route[1] ? 1 : 0);
    junction.classList.toggle("shadow-on", rs > 0.5 && rs < 0.95);
    const su = smooth(0.55, 0.8, rs);
    junctionShadow.setAttribute("cx", String(lerp(240, 168, su)));
    junctionShadow.setAttribute("cy", String(lerp(60, 118, su)));
    requestAnimationFrame(junctionTick);
  }

  /* ---------------- Typing intro ---------------- */

  let typingDone = false;
  if (typedEl) {
    const full = typedEl.textContent;
    typedEl.textContent = "";
    let i = 0;
    const finish = () => {
      typingDone = true;
      packet.classList.add("pulse");
      frame();
    };
    const typeNext = () => {
      if (typingDone) return;
      if (storyP > 0.04) {
        typedEl.textContent = full;
        finish();
        return;
      }
      typedEl.textContent = full.slice(0, ++i);
      if (i >= full.length) {
        finish();
      } else {
        window.setTimeout(typeNext, 26 + Math.random() * 46);
      }
    };
    window.setTimeout(typeNext, 700);
  }

  /* ---------------- Main frame loop ---------------- */

  let storyP = 0;
  let ticking = false;

  function frame() {
    ticking = false;
    const rect = story.getBoundingClientRect();
    const scrollable = story.offsetHeight - window.innerHeight;
    storyP = clamp01(-rect.top / Math.max(1, scrollable));

    const e = prog(storyP, ...PHASES.explode);
    const r = prog(storyP, ...PHASES.end);
    const focus = focusPos();

    for (let i = 0; i < plates.length; i++) {
      const sceneIdx = SCENES.findIndex((s) => s.plate === i);
      const sI = sceneIdx >= 0 ? prog(storyP, ...PHASES[SCENES[sceneIdx].name]) : 0;
      const nextScene = sceneIdx >= 0 && sceneIdx + 1 < SCENES.length ? SCENES[sceneIdx + 1] : null;
      const sNext = nextScene ? prog(storyP, ...PHASES[nextScene.name]) : 0;

      let T, O;
      if (i === 0) {
        const t = easeOut(e);
        T = lerpPt({ x: 0, y: 0, z: 0 }, exploded[0], t);
        T.rx = lerp(0, exploded[0].rx, t);
        T.ry = lerp(0, exploded[0].ry, t);
        O = 1;
      } else {
        const stagger = clamp01((e - i * 0.07) / 0.72);
        T = lerpPt(startStack[i], exploded[i], easeOut(stagger));
        T.rx = lerp(0, exploded[i].rx, easeOut(stagger));
        T.ry = lerp(0, exploded[i].ry, easeOut(stagger));
        O = stagger;
      }

      if (sceneIdx >= 0) {
        const f = smooth(0, 0.22, sI);
        const F = { ...focus, rx: lerp(exploded[i].rx, 0, f), ry: lerp(exploded[i].ry, 0, f) };
        T = lerpPt(T, F, f);
        T.rx = lerp(T.rx, 0, f);
        T.ry = lerp(T.ry, 0, f);
        O = lerp(O, 1, f);
      }
      if (nextScene) {
        const g = smooth(0, 0.3, sNext);
        const RA = { x: exploded[i].x - W * 0.05, y: exploded[i].y - H * 0.04, z: exploded[i].z - 140, rx: exploded[i].rx, ry: exploded[i].ry };
        T = lerpPt(T, RA, g);
        T.rx = lerp(T.rx, RA.rx, g);
        T.ry = lerp(T.ry, RA.ry, g);
        O = lerp(O, 0.18, g);
      }

      const activeScene = SCENES.find((s) => storyP >= PHASES[s.name][0] && storyP < PHASES[s.name][1]);
      if (activeScene && activeScene.plate !== i) {
        O = Math.min(O, activeScene.plate > i ? 0.4 : 0.18);
      }

      if (r > 0) {
        const c = smooth(0.02, 0.5, r);
        const endPos = i === 0 ? { x: 0, y: 0, z: 200 } : { x: 0, y: -i * 2, z: 60 - i * 16 };
        T = lerpPt(T, endPos, c);
        T.rx = lerp(T.rx, 0, c);
        T.ry = lerp(T.ry, 0, c);
        O = lerp(O, 1, smooth(0.02, 0.4, r));
        if (i > 0) O = lerp(O, 0, smooth(0.5, 0.78, r));
      }

      applyPlate(i, T, O);
    }

    plates[0].classList.toggle("done", r > 0.72);

    /* Signal line along the diagonal */
    if (signal) {
      const A = exploded[0];
      const B = exploded[5];
      const len = Math.hypot(B.x - A.x, B.y - A.y);
      const ang = (Math.atan2(B.y - A.y, B.x - A.x) * 180) / Math.PI;
      const vis = e * (1 - smooth(0.55, 0.85, r));
      signal.style.opacity = String(vis * 0.9);
      signal.style.width = `${len}px`;
      signal.style.transform = `translate(${A.x}px, ${A.y}px) rotate(${ang}deg)`;
    }

    /* Packet */
    if (packet) {
      const inputPos = { x: 0, y: (plateH[0] || 360) * FIT * 0.5 - 24 };
      const halfOf = (m) => ((plateH[m] || 360) * FIT) / 2 + 14;
      let pos = inputPos;
      let o = 0;
      if (e <= 0) {
        o = typingDone ? 1 : 0;
      } else if (r <= 0) {
        o = 1;
        const firstTop = { x: focus.x, y: focus.y - halfOf(1) };
        const activeScene = SCENES.find((s) => storyP >= PHASES[s.name][0] && storyP < PHASES[s.name][1]);
        if (!activeScene) {
          pos = lerpPt(inputPos, firstTop, easeOut(e));
        } else {
          const s = prog(storyP, ...PHASES[activeScene.name]);
          const m = activeScene.plate;
          const top = { x: focus.x, y: focus.y - halfOf(m) };
          const bottom = { x: focus.x, y: focus.y + halfOf(m) };
          const prevBottom = { x: focus.x, y: focus.y + halfOf(m - 1) };
          if (activeScene.name === "inspect") {
            pos = lerpPt(top, bottom, smooth(0.06, 0.94, s));
          } else if (activeScene.name === "govern") {
            const gateY = focus.y - ((plateH[3] || 360) * FIT) * 0.08;
            if (s < 0.08) pos = lerpPt(prevBottom, top, smooth(0, 0.08, s));
            else if (s < 0.3) pos = lerpPt(top, { x: focus.x, y: gateY }, smooth(0.08, 0.3, s));
            else if (s < 0.72) pos = { x: focus.x, y: gateY };
            else pos = lerpPt({ x: focus.x, y: gateY }, bottom, smooth(0.72, 0.94, s));
          } else if (activeScene.name === "observe") {
            if (s < 0.08) pos = lerpPt(prevBottom, top, smooth(0, 0.08, s));
            else pos = lerpPt(top, bottom, smooth(0.08, 0.4, s));
            o = 1 - smooth(0.4, 0.55, s);
          } else {
            if (s < 0.08) pos = lerpPt(prevBottom, top, smooth(0, 0.08, s));
            else pos = lerpPt(top, bottom, smooth(0.08, 0.94, s));
          }
        }
      }
      if (r > 0) o = 0;
      packet.style.opacity = String(clamp01(o));
      packet.classList.toggle("pulse", typingDone && e <= 0);
      if (pos) packet.style.transform = `translate(${pos.x}px, ${pos.y}px) translate(-50%, -50%)`;
    }

    /* Copy blocks */
    let activeCopy = "intro";
    if (storyP >= PHASES.end[0]) activeCopy = "end";
    else if (storyP >= PHASES.observe[0]) activeCopy = "observe";
    else if (storyP >= PHASES.execute[0]) activeCopy = "execute";
    else if (storyP >= PHASES.govern[0]) activeCopy = "govern";
    else if (storyP >= PHASES.route[0]) activeCopy = "route";
    else if (storyP >= PHASES.inspect[0]) activeCopy = "inspect";
    else if (storyP >= PHASES.explode[0]) activeCopy = "explode";
    copyBlocks.forEach((block) => block.classList.toggle("active", block.dataset.scene === activeCopy));

    /* INSPECT scan */
    if (scanline) {
      const s = prog(storyP, ...PHASES.inspect);
      const scan = smooth(0.28, 0.82, s);
      scanline.style.top = `${scan * 92 - 14}%`;
      inspectRows.forEach((row, idx) => {
        row.style.opacity = String(scan > idx / (inspectRows.length - 1) * 0.9 ? 1 : 0.22);
      });
    }

    /* GOVERN sequence */
    if (gate) {
      const s = prog(storyP, ...PHASES.govern);
      govRows.forEach((row, idx) => row.classList.toggle("show", s > 0.04 + idx * 0.1));
      gate.classList.toggle("closed", s > 0.28 && s < 0.72);
      approvalCard.classList.toggle("show", s > 0.4 && s < 0.9);
      approveBtn.classList.toggle("pressed", s > 0.55 && s < 0.72);
      govCleared.classList.toggle("show", s > 0.72);
    }

    /* EXECUTE sequence */
    if (chamber) {
      const s = prog(storyP, ...PHASES.execute);
      chamber.classList.toggle("sealed", s > 0.2);
      runSteps.forEach((step, idx) => step.classList.toggle("show", s > 0.34 + idx * 0.13));
    }

    /* OBSERVE timeline */
    {
      const s = prog(storyP, ...PHASES.observe);
      tlRows.forEach((row, idx) => row.classList.toggle("show", s > 0.05 + (idx / (tlRows.length - 1)) * 0.8));
    }
  }

  function onScroll() {
    if (!ticking) {
      ticking = true;
      requestAnimationFrame(frame);
    }
  }

  window.addEventListener("scroll", onScroll, { passive: true });
  let resizeTimer;
  const remeasure = () => {
    window.clearTimeout(resizeTimer);
    resizeTimer = window.setTimeout(() => {
      measure();
      frame();
    }, 120);
  };
  window.addEventListener("resize", remeasure);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) remeasure();
  });
  if (document.fonts && document.fonts.ready) {
    document.fonts.ready.then(() => {
      measure();
      frame();
    });
  }

  measure();
  frame();
  requestAnimationFrame(junctionTick);
})();
