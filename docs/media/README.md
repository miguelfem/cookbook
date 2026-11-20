# Visuals for the CookBook repository

The README reserves the following recording slots. Walkthrough/native screenshots
remain planned. Recorded [capacity](../evaluations/2026-09-22-worker-capacity.md)
and [client/routing charts](../evaluations/2026-09-22-readme-candidate.md) now link
to their measured data, failed attempts and limits. The current architecture
diagram is authored directly in the README.

## Walkthrough

**Slot:** a captioned 45–60 second recording, with a still image linking to the video.

Show a fresh project in a supported agent → ask for a recipe → inspect the saved steps in CookBook → run it with new input → open the result. Use the account-free save-note or synthetic report recipe. Keep any edits to waiting time visible; do not stage success. Add a short text transcript so the walkthrough works without video.

## Native app

**Slot:** one wide screenshot of the current recipe library, followed by a small run-detail image in the usage guide.

Use the actual packaged Tauri app with disposable synthetic state. Show the recipe name, readable steps, completed status and resulting artifact. Include the app version, source/package identity, platform and capture date in the caption. Avoid screenshots of the owner's workspace, keys, local attachment paths or personal project names.

## Results charts

**Slots:** quality versus cost; total cost as repetitions increase; first-use completion and time.

Create these only from an accepted [evaluation report](../evaluations/README.md). Link each chart to its sample counts, underlying reviewed aggregate data and method. Separate setup from repeat execution; include failures and uncertainty. Use legible labels, accessible contrast and descriptions. Unknown cost remains unknown. Until data exists, retain the text placeholder instead of inventing a chart.

## Adding an asset

Place reviewed small images in this directory and use relative Markdown links with descriptive alt text. Use a stable, owner-approved host for larger recordings and keep a local poster image. Record filename, creator, date, source commit, synthetic/live scope and any third-party attribution here when adding it. Ask participants for permission before publishing their recording or identifiable feedback.

Replace the corresponding README status only when the asset is checked and its link works. Keep editable source for diagrams/charts alongside the image or linked from its evaluation report.
