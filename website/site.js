const menuButton = document.querySelector(".menu-toggle");
const nav = document.querySelector(".site-nav");

if (nav && !nav.querySelector('a[href="benchmarks.html"]')) {
  const benchmarkLink = document.createElement("a");
  benchmarkLink.href = "benchmarks.html";
  benchmarkLink.textContent = "Benchmarks";
  nav.querySelector('a[href="privacy.html"]')?.before(benchmarkLink);
}

if (menuButton && nav) {
  menuButton.addEventListener("click", () => {
    const open = nav.classList.toggle("open");
    menuButton.setAttribute("aria-expanded", String(open));
  });
}

const copyIcon = '<svg aria-hidden="true" viewBox="0 0 24 24"><rect x="9" y="9" width="11" height="11" rx="1"></rect><path d="M15 9V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v9a1 1 0 0 0 1 1h4"></path></svg>';

document.querySelectorAll(".copy").forEach((button) => {
  button.innerHTML = copyIcon;
  button.addEventListener("click", async () => {
    const command = button.closest(".command-row")?.querySelector("code")?.textContent;
    if (!command) return;
    try {
      await navigator.clipboard.writeText(command.trim());
      button.dataset.copied = "true";
      button.setAttribute("aria-label", "Command copied");
      window.setTimeout(() => {
        delete button.dataset.copied;
        button.setAttribute("aria-label", "Copy command");
      }, 1600);
    } catch {
      button.setAttribute("aria-label", "Select command text to copy");
    }
  });
});

const githubStars = document.querySelectorAll("[data-github-stars]");

if (githubStars.length) {
  const cacheKey = "kerna-github-stars";
  const cacheLifetime = 15 * 60 * 1000;

  const showStars = (count) => {
    const formatted = Number(count).toLocaleString();
    githubStars.forEach((starCount) => {
      starCount.textContent = formatted;
      starCount.closest(".github-link")?.setAttribute("aria-label", `View Kerna on GitHub. ${formatted} stars.`);
    });
  };

  try {
    const cached = JSON.parse(window.localStorage.getItem(cacheKey) || "null");
    if (cached && Date.now() - cached.savedAt < cacheLifetime) showStars(cached.count);
  } catch {}

  fetch("https://api.github.com/repos/premxai/kerna", { headers: { Accept: "application/vnd.github+json" } })
    .then((response) => (response.ok ? response.json() : Promise.reject(response)))
    .then((repository) => {
      showStars(repository.stargazers_count);
      try {
        window.localStorage.setItem(cacheKey, JSON.stringify({ count: repository.stargazers_count, savedAt: Date.now() }));
      } catch {}
    })
    .catch(() => {});
}
