import init, { analyze_logs } from './pkg/ctrlb_decompose.js';

const btn = document.getElementById('analyze-btn');
const logInput = document.getElementById('log-input');
const output = document.getElementById('output');
const statsEl = document.getElementById('stats');
const formatSelect = document.getElementById('format-select');
const topN = document.getElementById('top-n');
const context = document.getElementById('context');
const fileInput = document.getElementById('file-input');
const dropOverlay = document.getElementById('drop-overlay');

async function main() {
    await init();
    btn.disabled = false;
    btn.textContent = 'Analyze';

    btn.addEventListener('click', runAnalysis);

    // Ctrl/Cmd+Enter shortcut
    logInput.addEventListener('keydown', (e) => {
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
            e.preventDefault();
            runAnalysis();
        }
    });

    // File input
    fileInput.addEventListener('change', (e) => {
        const file = e.target.files[0];
        if (file) loadFile(file);
    });

    // Drag and drop
    let dragCounter = 0;

    document.addEventListener('dragenter', (e) => {
        e.preventDefault();
        dragCounter++;
        dropOverlay.classList.add('active');
    });

    document.addEventListener('dragleave', (e) => {
        e.preventDefault();
        dragCounter--;
        if (dragCounter === 0) dropOverlay.classList.remove('active');
    });

    document.addEventListener('dragover', (e) => e.preventDefault());

    document.addEventListener('drop', (e) => {
        e.preventDefault();
        dragCounter = 0;
        dropOverlay.classList.remove('active');
        const file = e.dataTransfer.files[0];
        if (file) loadFile(file);
    });
}

function loadFile(file) {
    const reader = new FileReader();
    reader.onload = () => {
        logInput.value = reader.result;
    };
    reader.readAsText(file);
}

function runAnalysis() {
    const input = logInput.value;
    if (!input.trim()) {
        output.textContent = 'No input provided.';
        output.classList.remove('has-content');
        statsEl.textContent = '';
        return;
    }

    const format = formatSelect.value;
    const top = parseInt(topN.value) || 20;
    const ctx = parseInt(context.value) || 0;

    btn.disabled = true;
    btn.textContent = 'Analyzing...';

    // Use setTimeout to let the UI update before blocking on WASM
    setTimeout(() => {
        try {
            const start = performance.now();
            let result = analyze_logs(input, format, top, ctx);
            const elapsed = (performance.now() - start).toFixed(0);

            // Pretty-print JSON
            if (format === 'json') {
                try {
                    result = JSON.stringify(JSON.parse(result), null, 2);
                } catch (_) { /* already formatted or parse error, show raw */ }
            }

            const lineCount = input.split('\n').filter(l => l.trim()).length;
            output.textContent = result;
            output.classList.add('has-content');
            statsEl.textContent = `${lineCount.toLocaleString()} lines analyzed in ${elapsed}ms`;
        } catch (err) {
            output.textContent = `Error: ${err.message || err}`;
            output.classList.remove('has-content');
            statsEl.textContent = '';
        } finally {
            btn.disabled = false;
            btn.textContent = 'Analyze';
        }
    }, 10);
}

main();
