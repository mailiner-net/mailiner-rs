import { test, expect, type Locator, type Page } from '@playwright/test';
import { composeButton, composeOverlay, gotoMail, seedAccount } from './helpers';

/**
 * Bold / italic / underline run against the open shadow root, where
 * `document.execCommand` does not toggle. These cover issue #245.
 */

test('bold, italic, and underline unwrap a formatted selection', async ({ page }) => {
  const compose = await openRichCompose(page);
  const host = editorHost(compose);

  await setEditorHtml(host, '<p><strong>world</strong></p>');
  await selectPlain(host, 0, 5);
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await expectMarks(host, ['', '', '', '', '']);

  await setEditorHtml(host, '<p><b>hello</b></p>');
  await selectPlain(host, 0, 5);
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await expectMarks(host, ['', '', '', '', '']);

  await setEditorHtml(host, '<p><em>hello</em></p>');
  await selectPlain(host, 0, 5);
  await compose.getByRole('button', { name: 'Italic', exact: true }).click();
  await expectMarks(host, ['', '', '', '', '']);

  await setEditorHtml(host, '<p><i>hello</i></p>');
  await selectPlain(host, 0, 5);
  await compose.getByRole('button', { name: 'Italic', exact: true }).click();
  await expectMarks(host, ['', '', '', '', '']);

  await setEditorHtml(host, '<p><u>hello</u></p>');
  await selectPlain(host, 0, 5);
  await compose.getByRole('button', { name: 'Underline', exact: true }).click();
  await expectMarks(host, ['', '', '', '', '']);
});

test('a partial selection splits the mark and leaves the rest', async ({ page }) => {
  const compose = await openRichCompose(page);
  const host = editorHost(compose);
  await setEditorHtml(host, '<p><strong>abcdef</strong></p>');
  await selectPlain(host, 2, 4);
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await expectMarks(host, ['strong', 'strong', '', '', 'strong', 'strong']);
  await expectNoNestedAlias(host, ['strong', 'b']);
});

test('mixed content toggles without nesting the same mark', async ({ page }) => {
  const compose = await openRichCompose(page);
  const host = editorHost(compose);

  await setEditorHtml(host, '<p>hello <strong>world</strong></p>');
  await selectPlain(host, 0, 11);
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await expectMarks(host, Array(11).fill('strong'));
  await expectNoNestedAlias(host, ['strong', 'b']);

  await setEditorHtml(host, '<p><strong>hello <em>world</em>!</strong></p>');
  await selectPlain(host, 0, 12);
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await expectMarks(host, [
    '',
    '',
    '',
    '',
    '',
    '',
    'em',
    'em',
    'em',
    'em',
    'em',
    '',
  ]);
  await expectNoNestedAlias(host, ['strong', 'b']);

  await setEditorHtml(host, '<p><strong>one</strong></p><p><strong>two</strong></p>');
  await selectPlain(host, 0, 6);
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await expectMarks(host, ['', '', '', '', '', '']);

  // Unwrapping bold around "cd" splits the strong through the em. The right
  // strong used to keep an empty em that only held the end marker.
  await setEditorHtml(host, '<p><strong>ab<em>cd</em>ef</strong></p>');
  await selectPlain(host, 2, 4);
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await expectMarks(host, ['strong', 'strong', 'em', 'em', 'strong', 'strong']);
  await expectNoEmptyInline(host);
});

test('a collapsed caret still enters the mark and a second click leaves it', async ({ page }) => {
  const compose = await openRichCompose(page);
  const host = editorHost(compose);
  await host.click();
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await page.keyboard.type('A');
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await page.keyboard.type('B');
  await expectMarks(host, ['strong', '']);
});

async function openRichCompose(page: Page) {
  await seedAccount(page);
  await gotoMail(page);
  await composeButton(page).click();
  const compose = composeOverlay(page);
  await compose.getByRole('button', { name: 'Rich', exact: true }).click();
  const host = editorHost(compose);
  await expect(host).toBeVisible();
  await expect
    .poll(() =>
      host.evaluate((el) =>
        Boolean(el.shadowRoot?.getElementById('mlnr-compose-editable')),
      ),
    )
    .toBe(true);
  return compose;
}

function editorHost(compose: Locator) {
  return compose.locator('#mailiner-compose-editor');
}

async function setEditorHtml(host: Locator, html: string) {
  await host.evaluate((el, next) => {
    const edit = el.shadowRoot?.getElementById('mlnr-compose-editable');
    if (!edit) {
      throw new Error('compose editor is not mounted');
    }
    edit.innerHTML = next;
  }, html);
}

async function selectPlain(host: Locator, start: number, end: number) {
  const selected = await host.evaluate(
    (el, span) => {
      const edit = el.shadowRoot?.getElementById('mlnr-compose-editable');
      if (!edit) {
        throw new Error('compose editor is not mounted');
      }
      edit.focus();
      const walker = edit.ownerDocument.createTreeWalker(edit, NodeFilter.SHOW_TEXT);
      let seen = 0;
      let startNode: Text | null = null;
      let startOffset = 0;
      let endNode: Text | null = null;
      let endOffset = 0;
      while (walker.nextNode()) {
        const text = walker.currentNode as Text;
        const len = text.length;
        if (!startNode && span.start <= seen + len) {
          startNode = text;
          startOffset = span.start - seen;
        }
        if (!endNode && span.end <= seen + len) {
          endNode = text;
          endOffset = span.end - seen;
        }
        seen += len;
      }
      if (!startNode || !endNode) {
        throw new Error(`cannot select ${span.start}-${span.end} in ${seen} chars`);
      }
      const range = edit.ownerDocument.createRange();
      range.setStart(startNode, startOffset);
      range.setEnd(endNode, endOffset);
      const root = edit.getRootNode() as ShadowRoot & {
        getSelection?: () => Selection | null;
      };
      const sel = root.getSelection?.();
      if (!sel) {
        throw new Error('shadow root has no selection');
      }
      sel.removeAllRanges();
      sel.addRange(range);
      return sel.toString();
    },
    { start, end },
  );
  expect(selected).toBeTruthy();
}

async function readMarks(host: Locator): Promise<string[]> {
  return host.evaluate((el) => {
    const edit = el.shadowRoot?.getElementById('mlnr-compose-editable');
    if (!edit) {
      return [];
    }
    if (edit.querySelector('[data-mlnr-fmt]')) {
      throw new Error(`format markers leaked: ${edit.innerHTML}`);
    }
    const interesting = new Set(['strong', 'b', 'em', 'i', 'u']);
    const marks: string[] = [];
    const walk = (node: Node, stack: string[]) => {
      if (node.nodeType === Node.TEXT_NODE) {
        for (const ch of node.textContent ?? '') {
          if (ch !== '\u200b') {
            marks.push(stack.join('+'));
          }
        }
        return;
      }
      const name =
        node.nodeType === Node.ELEMENT_NODE ? (node as Element).tagName.toLowerCase() : '';
      const next = interesting.has(name) ? [...stack, name] : stack;
      for (const child of [...node.childNodes]) {
        walk(child, next);
      }
    };
    walk(edit, []);
    return marks;
  });
}

async function expectMarks(host: Locator, expected: string[]) {
  await expect.poll(async () => readMarks(host)).toEqual(expected);
}

async function expectNoEmptyInline(host: Locator) {
  const empty = await host.evaluate((el) => {
    const edit = el.shadowRoot?.getElementById('mlnr-compose-editable');
    if (!edit) {
      return ['missing editor'];
    }
    const tags = ['a', 'b', 'code', 'em', 'i', 'span', 'strong', 'u'];
    const found: string[] = [];
    for (const node of edit.querySelectorAll(tags.join(','))) {
      if (node.childNodes.length === 0) {
        found.push(node.outerHTML);
      }
    }
    return found;
  });
  expect(empty).toEqual([]);
}

async function expectNoNestedAlias(host: Locator, aliases: string[]) {
  const nested = await host.evaluate((el, names) => {
    const edit = el.shadowRoot?.getElementById('mlnr-compose-editable');
    if (!edit) {
      return true;
    }
    const alias = new Set(names);
    for (const node of edit.querySelectorAll(names.join(','))) {
      let parent = node.parentElement;
      while (parent && parent !== edit) {
        if (alias.has(parent.tagName.toLowerCase())) {
          return true;
        }
        parent = parent.parentElement;
      }
    }
    return false;
  }, aliases);
  expect(nested).toBe(false);
}
