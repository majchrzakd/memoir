<script lang="ts">
  // M0 placeholder layout; the real tree and editor arrive in M3.
</script>

<div class="layout">
  <nav class="tree" aria-label="Notebooks">
    <p class="muted">No notes yet</p>
  </nav>
  <main class="editor">
    <p class="muted">Memoir</p>
  </main>
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 280px 1fr;
    height: 100vh;
  }

  .tree {
    border-right: 1px solid var(--border);
    background: var(--bg-sidebar);
    padding: 12px;
    overflow: auto;
  }

  .editor {
    padding: 24px 32px;
    overflow: auto;
  }

  .muted {
    color: var(--text-muted);
  }
</style>
