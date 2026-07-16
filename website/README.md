# Kerna website

Static official product site for the first Kerna cohort. It has no build step;
serve `website/` from any static host.

For a local preview:

```powershell
python -m http.server 4173 --directory website
```

Then open `http://localhost:4173`.

The site is intentionally explicit that the first Kerna release is local-first
and does not require login. Download totals are not described as user counts.
