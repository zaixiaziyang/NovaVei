export type MarkdownBlock =
  | { type: "paragraph"; lines: string[] }
  | { type: "heading"; level: number; text: string }
  | { type: "code"; language: string; code: string }
  | { type: "quote"; lines: string[] }
  | { type: "list"; ordered: boolean; items: string[] }
  | {
      type: "table";
      headers: string[];
      alignments: TableAlignment[];
      rows: string[][];
    }
  | { type: "rule" };

type TableAlignment = "left" | "center" | "right" | undefined;
type ParsedTable = {
  block: Extract<MarkdownBlock, { type: "table" }>;
  nextIndex: number;
};

type MarkdownRenderState = {
  source: string;
  /** Source through the last blank-line-delimited, immutable Markdown block. */
  committedLength: number;
  /** DOM for the single growing tail, replaced on the next streaming update. */
  tailNodes: Node[];
  /** A tail whose DOM can be extended without parsing the accumulated text. */
  appendableTail?: AppendableTail;
};

type FenceLineProgress =
  | { type: "indent"; spaces: number }
  | { type: "marker"; markers: number }
  | { type: "trailing"; markers: number }
  | { type: "invalid" };

type OpenCodeFence = {
  marker: string;
  width: number;
  end: RegExp;
  lineProgress: FenceLineProgress;
};

type AppendableTail =
  | { type: "code"; pre: HTMLPreElement; fence: OpenCodeFence }
  | { type: "plain-paragraph"; paragraph: HTMLParagraphElement };

const renderedMarkdown = new WeakMap<HTMLElement, MarkdownRenderState>();

function normaliseMarkdownSource(source: string) {
  return source.replace(/\r\n?/g, "\n");
}

function initialFenceLineProgress(): FenceLineProgress {
  return { type: "indent", spaces: 0 };
}

function advanceFenceLineProgress(
  progress: FenceLineProgress,
  character: string,
  marker: string,
  width: number,
): FenceLineProgress {
  if (progress.type === "invalid") return progress;
  if (progress.type === "indent") {
    if (character === " " && progress.spaces < 3) {
      return { type: "indent", spaces: progress.spaces + 1 };
    }
    if (character === marker) return { type: "marker", markers: 1 };
    return { type: "invalid" };
  }
  if (progress.type === "marker") {
    if (character === marker) {
      return { type: "marker", markers: progress.markers + 1 };
    }
    if (/\s/.test(character) && progress.markers >= width) {
      return { type: "trailing", markers: progress.markers };
    }
    return { type: "invalid" };
  }
  if (/\s/.test(character)) return progress;
  return { type: "invalid" };
}

function isCompleteFenceLine(progress: FenceLineProgress, width: number) {
  return (
    (progress.type === "marker" || progress.type === "trailing") &&
    progress.markers >= width
  );
}

/**
 * Returns the final unclosed code fence only after its opening line is
 * complete. Until then, streamed language text can still change the header
 * and must use the ordinary renderer.
 */
function findOpenCodeFence(source: string): OpenCodeFence | undefined {
  const lines = source.split("\n");
  let open:
    | { marker: string; width: number; end: RegExp; lineIndex: number }
    | undefined;

  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (open) {
      if (open.end.test(line)) open = undefined;
      continue;
    }
    const fence = line.match(/^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)\s*$/);
    if (!fence) continue;
    open = {
      marker: fence[1][0],
      width: fence[1].length,
      end: new RegExp(`^ {0,3}${fence[1][0]}{${fence[1].length},}\\s*$`),
      lineIndex: index,
    };
  }

  // Without the opening line's newline, an append can still be part of the
  // language token rather than code text.
  if (!open || open.lineIndex === lines.length - 1) return undefined;

  let lineProgress = initialFenceLineProgress();
  for (let index = open.lineIndex + 1; index < lines.length; index += 1) {
    for (const character of lines[index]) {
      lineProgress = advanceFenceLineProgress(
        lineProgress,
        character,
        open.marker,
        open.width,
      );
    }
    if (index + 1 < lines.length) lineProgress = initialFenceLineProgress();
  }
  return { ...open, lineProgress };
}

/** Returns false if the appended characters close the currently open fence. */
function extendOpenCodeFence(fence: OpenCodeFence, appendedSource: string) {
  let progress = fence.lineProgress;
  for (const character of appendedSource) {
    if (character === "\n") {
      if (isCompleteFenceLine(progress, fence.width)) return false;
      progress = initialFenceLineProgress();
      continue;
    }
    progress = advanceFenceLineProgress(
      progress,
      character,
      fence.marker,
      fence.width,
    );
  }
  if (isCompleteFenceLine(progress, fence.width)) return false;
  fence.lineProgress = progress;
  return true;
}

// Restrict the paragraph fast path to text which cannot gain block or inline
// Markdown meaning after another same-line append. This deliberately leaves
// punctuation-heavy prose on the normal (correctness-first) path.
function isPlainAppendableText(source: string) {
  return !/[\r\n`[\]()*~\\#>+\-_|.]/.test(source);
}

/**
 * A blank line closes the preceding Markdown block in the supported subset.
 * Everything after the final boundary remains mutable while a provider stream
 * is still appending tokens.
 */
function lastCompletedBlockBoundary(source: string) {
  const lines = source.split("\n");
  let offset = 0;
  let boundary = 0;
  let fenceEnd: RegExp | undefined;
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (fenceEnd) {
      if (fenceEnd.test(line)) fenceEnd = undefined;
    } else {
      const fence = line.match(/^ {0,3}(`{3,}|~{3,})\s*[^\s`]*\s*$/);
      if (fence) {
        fenceEnd = new RegExp(
          `^ {0,3}${fence[1][0]}{${fence[1].length},}\\s*$`,
        );
      } else if (!line.trim() && index + 1 < lines.length) {
        // `offset + line.length + 1` is just after the blank line's newline.
        boundary = offset + line.length + 1;
      }
    }
    offset += line.length + (index + 1 < lines.length ? 1 : 0);
  }
  return boundary;
}

function isBlockStart(line: string) {
  return /^(?: {0,3}(?:#{1,6}\s+|```|~~~|>|[-+*]\s+|\d+[.)]\s+)| {0,3}(?:-{3,}|_{3,}|\*{3,})\s*$)/.test(
    line,
  );
}

/** Split a GFM table row without treating an escaped pipe as a column boundary. */
function parseTableCells(line: string) {
  if (!line.includes("|")) return undefined;
  const cells: string[] = [];
  let currentCell = "";
  let characterIndex = 0;

  while (characterIndex < line.length) {
    const character = line[characterIndex];
    if (character === "\\" && line[characterIndex + 1] === "|") {
      currentCell += "|";
      characterIndex += 2;
      continue;
    }
    if (character === "|") {
      cells.push(currentCell.trim());
      currentCell = "";
      characterIndex += 1;
      continue;
    }
    currentCell += character;
    characterIndex += 1;
  }
  cells.push(currentCell.trim());

  if (line.trimStart().startsWith("|")) cells.shift();
  if (line.trimEnd().endsWith("|")) cells.pop();
  return cells.length ? cells : undefined;
}

function tableAlignment(value: string): TableAlignment {
  if (!/^:?-{3,}:?$/.test(value)) return undefined;
  const startsWithColon = value.startsWith(":");
  const endsWithColon = value.endsWith(":");
  if (startsWithColon && endsWithColon) return "center";
  if (endsWithColon) return "right";
  return "left";
}

function parseTable(
  lines: string[],
  startIndex: number,
): ParsedTable | undefined {
  const headers = parseTableCells(lines[startIndex] ?? "");
  const separatorCells = parseTableCells(lines[startIndex + 1] ?? "");
  if (
    !headers?.length ||
    headers.some((header) => !header) ||
    !separatorCells ||
    separatorCells.length !== headers.length
  ) {
    return undefined;
  }

  const alignments = separatorCells.map(tableAlignment);
  if (alignments.some((alignment) => alignment === undefined)) return undefined;

  const rows: string[][] = [];
  let index = startIndex + 2;
  while (index < lines.length) {
    const cells = parseTableCells(lines[index]);
    if (!cells) break;
    rows.push(
      Array.from(
        { length: headers.length },
        (_, columnIndex) => cells[columnIndex] ?? "",
      ),
    );
    index += 1;
  }

  return {
    block: { type: "table", headers, alignments, rows },
    nextIndex: index,
  };
}

/**
 * A deliberately small Markdown subset for provider output. Raw HTML is
 * never interpreted; unsupported syntax remains visible as ordinary text.
 */
export function parseMarkdown(source: string): MarkdownBlock[] {
  const lines = normaliseMarkdownSource(source).split("\n");
  const blocks: MarkdownBlock[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (!line.trim()) {
      index += 1;
      continue;
    }

    const table = parseTable(lines, index);
    if (table) {
      blocks.push(table.block);
      index = table.nextIndex;
      continue;
    }

    const fence = line.match(/^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)\s*$/);
    if (fence) {
      const marker = fence[1][0];
      const width = fence[1].length;
      const language = fence[2].slice(0, 48);
      const fenceEnd = new RegExp(`^ {0,3}${marker}{${width},}\\s*$`);
      const body: string[] = [];
      index += 1;
      while (index < lines.length && !fenceEnd.test(lines[index])) {
        body.push(lines[index]);
        index += 1;
      }
      if (index < lines.length) index += 1;
      blocks.push({ type: "code", language, code: body.join("\n") });
      continue;
    }

    const heading = line.match(/^ {0,3}(#{1,6})\s+(.+?)\s*#*\s*$/);
    if (heading) {
      blocks.push({
        type: "heading",
        level: heading[1].length,
        text: heading[2],
      });
      index += 1;
      continue;
    }

    if (/^ {0,3}(?:-{3,}|_{3,}|\*{3,})\s*$/.test(line)) {
      blocks.push({ type: "rule" });
      index += 1;
      continue;
    }

    const quote = line.match(/^ {0,3}>\s?(.*)$/);
    if (quote) {
      const quoteLines: string[] = [];
      while (index < lines.length) {
        const match = lines[index].match(/^ {0,3}>\s?(.*)$/);
        if (!match) break;
        quoteLines.push(match[1]);
        index += 1;
      }
      blocks.push({ type: "quote", lines: quoteLines });
      continue;
    }

    const unordered = line.match(/^ {0,3}[-+*]\s+(.+)$/);
    const ordered = line.match(/^ {0,3}\d+[.)]\s+(.+)$/);
    if (unordered || ordered) {
      const isOrdered = Boolean(ordered);
      const items: string[] = [];
      while (index < lines.length) {
        const match = isOrdered
          ? lines[index].match(/^ {0,3}\d+[.)]\s+(.+)$/)
          : lines[index].match(/^ {0,3}[-+*]\s+(.+)$/);
        if (!match) break;
        items.push(match[1]);
        index += 1;
      }
      blocks.push({ type: "list", ordered: isOrdered, items });
      continue;
    }

    const paragraph: string[] = [line];
    index += 1;
    while (
      index < lines.length &&
      lines[index].trim() &&
      !isBlockStart(lines[index]) &&
      !parseTable(lines, index)
    ) {
      paragraph.push(lines[index]);
      index += 1;
    }
    blocks.push({ type: "paragraph", lines: paragraph });
  }

  return blocks;
}

type InlineKind = "code" | "image" | "link" | "strong" | "strike" | "em";
type InlineMatch = { kind: InlineKind; match: RegExpMatchArray };

function nextInlineMatch(source: string): InlineMatch | undefined {
  const candidates: InlineMatch[] = [];
  const add = (kind: InlineKind, match: RegExpMatchArray | null) => {
    if (match) candidates.push({ kind, match });
  };
  add("code", source.match(/`([^`\n]+)`/));
  add("image", source.match(/!\[([^\]\n]*)\]\(([^\s)]+)(?:\s+"[^"]*")?\)/));
  add("link", source.match(/\[([^\]\n]+)\]\(([^\s)]+)(?:\s+"[^"]*")?\)/));
  add("strong", source.match(/\*\*([^\n]+?)\*\*/));
  add("strike", source.match(/~~([^\n]+?)~~/));
  add("em", source.match(/(?<!\*)\*([^*\n]+)\*(?!\*)/));
  candidates.sort(
    (left, right) => (left.match.index ?? 0) - (right.match.index ?? 0),
  );
  return candidates[0];
}

function safeLinkTarget(value: string) {
  const target = value.trim();
  if (/^(?:https?:|mailto:)/i.test(target)) return target;
  return undefined;
}

function safeImageTarget(value: string) {
  const target = value.trim();
  if (/^https:/i.test(target)) return target;
  if (/^(?:asset:|blob:)/i.test(target)) return target;
  if (/^http:\/\/asset\.localhost(?:[:/]|$)/i.test(target)) return target;
  if (/^data:image\/(?:avif|gif|jpe?g|png|webp);/i.test(target)) return target;
  return undefined;
}

function appendInline(parent: HTMLElement, source: string, depth = 0) {
  if (!source || depth > 8) {
    parent.appendChild(document.createTextNode(source));
    return;
  }
  let rest = source;
  while (rest) {
    const candidate = nextInlineMatch(rest);
    if (!candidate) {
      parent.appendChild(document.createTextNode(rest));
      break;
    }
    const offset = candidate.match.index ?? 0;
    if (offset)
      parent.appendChild(document.createTextNode(rest.slice(0, offset)));
    const [token] = candidate.match;
    let inline: HTMLElement | undefined;
    if (candidate.kind === "code") {
      inline = document.createElement("code");
      inline.textContent = candidate.match[1];
    } else if (candidate.kind === "image") {
      const src = safeImageTarget(candidate.match[2]);
      if (src) {
        const image = document.createElement("img");
        image.className = "markdown-image";
        image.src = src;
        image.alt = candidate.match[1] || "";
        image.loading = "lazy";
        image.decoding = "async";
        image.referrerPolicy = "no-referrer";
        inline = image;
      }
    } else if (candidate.kind === "link") {
      const href = safeLinkTarget(candidate.match[2]);
      if (href) {
        const link = document.createElement("a");
        link.href = href;
        appendInline(link, candidate.match[1], depth + 1);
        if (/^https?:/i.test(href)) {
          link.target = "_blank";
          link.rel = "noopener noreferrer";
        }
        inline = link;
      }
    } else {
      inline = document.createElement(
        candidate.kind === "strong"
          ? "strong"
          : candidate.kind === "strike"
            ? "s"
            : "em",
      );
      const content =
        candidate.kind === "em" ? candidate.match[1] : candidate.match[1];
      appendInline(inline, content, depth + 1);
    }
    if (inline) parent.appendChild(inline);
    else parent.appendChild(document.createTextNode(token));
    rest = rest.slice(offset + token.length);
  }
}

function appendLines(parent: HTMLElement, lines: string[]) {
  lines.forEach((line, index) => {
    if (index) parent.appendChild(document.createElement("br"));
    appendInline(parent, line);
  });
}

function renderBlock(block: MarkdownBlock) {
  if (block.type === "code") {
    const wrapper = document.createElement("div");
    wrapper.className = "code-block";
    const header = document.createElement("header");
    const language = document.createElement("span");
    language.textContent = block.language || "code";
    const copy = document.createElement("button");
    copy.type = "button";
    copy.dataset.historyAction = "copy-code";
    copy.textContent = "复制";
    copy.setAttribute(
      "aria-label",
      `复制${block.language ? ` ${block.language}` : ""}代码`,
    );
    const pre = document.createElement("pre");
    pre.textContent = block.code;
    header.append(language, copy);
    wrapper.append(header, pre);
    return wrapper;
  }
  if (block.type === "heading") {
    const heading = document.createElement(
      `h${Math.max(2, Math.min(6, block.level + 1))}`,
    );
    appendInline(heading, block.text);
    return heading;
  }
  if (block.type === "list") {
    const list = document.createElement(block.ordered ? "ol" : "ul");
    for (const value of block.items) {
      const item = document.createElement("li");
      appendInline(item, value);
      list.appendChild(item);
    }
    return list;
  }
  if (block.type === "quote") {
    const quote = document.createElement("blockquote");
    appendLines(quote, block.lines);
    return quote;
  }
  if (block.type === "table") {
    const wrapper = document.createElement("div");
    wrapper.className = "markdown-table-wrap";
    wrapper.setAttribute("role", "region");
    wrapper.setAttribute("aria-label", "Markdown 表格");
    const table = document.createElement("table");
    table.className = "markdown-table";
    const tableHead = document.createElement("thead");
    const headerRow = document.createElement("tr");
    block.headers.forEach((header, columnIndex) => {
      const headerCell = document.createElement("th");
      headerCell.scope = "col";
      const alignment = block.alignments[columnIndex];
      if (alignment) headerCell.dataset.align = alignment;
      appendInline(headerCell, header);
      headerRow.appendChild(headerCell);
    });
    tableHead.appendChild(headerRow);

    const tableBody = document.createElement("tbody");
    block.rows.forEach((row) => {
      const tableRow = document.createElement("tr");
      row.forEach((value, columnIndex) => {
        const cell = document.createElement("td");
        const alignment = block.alignments[columnIndex];
        if (alignment) cell.dataset.align = alignment;
        appendInline(cell, value);
        tableRow.appendChild(cell);
      });
      tableBody.appendChild(tableRow);
    });
    table.append(tableHead, tableBody);
    wrapper.appendChild(table);
    return wrapper;
  }
  if (block.type === "rule") return document.createElement("hr");
  const paragraph = document.createElement("p");
  appendLines(paragraph, block.lines);
  return paragraph;
}

export function renderMarkdown(target: HTMLElement, source: string) {
  const normalisedSource = normaliseMarkdownSource(source);
  let state = renderedMarkdown.get(target);
  if (state?.source === normalisedSource) return;

  // Static/history content and protocol-strip rewrites can replace arbitrary
  // text. Only an append-only stream is safe to render incrementally.
  if (!state || !normalisedSource.startsWith(state.source)) {
    target.replaceChildren();
    state = {
      source: "",
      committedLength: 0,
      tailNodes: [],
    };
    renderedMarkdown.set(target, state);
  } else {
    const appendedSource = normalisedSource.slice(state.source.length);
    if (state.appendableTail?.type === "code") {
      if (extendOpenCodeFence(state.appendableTail.fence, appendedSource)) {
        state.appendableTail.pre.appendChild(
          document.createTextNode(appendedSource),
        );
        state.source = normalisedSource;
        return;
      }
    } else if (
      state.appendableTail?.type === "plain-paragraph" &&
      isPlainAppendableText(appendedSource)
    ) {
      state.appendableTail.paragraph.appendChild(
        document.createTextNode(appendedSource),
      );
      state.source = normalisedSource;
      return;
    }
    for (const node of state.tailNodes) node.parentNode?.removeChild(node);
    state.appendableTail = undefined;
  }

  const pendingSource = normalisedSource.slice(state.committedLength);
  const completedLength = lastCompletedBlockBoundary(pendingSource);
  if (completedLength) {
    const completedFragment = document.createDocumentFragment();
    for (const block of parseMarkdown(pendingSource.slice(0, completedLength)))
      completedFragment.appendChild(renderBlock(block));
    target.appendChild(completedFragment);
    state.committedLength += completedLength;
  }

  const tailSource = normalisedSource.slice(state.committedLength);
  const tailBlocks = parseMarkdown(tailSource);
  const fragment = document.createDocumentFragment();
  for (const block of tailBlocks) fragment.appendChild(renderBlock(block));
  state.tailNodes = [...fragment.childNodes];
  target.appendChild(fragment);
  const finalBlock = tailBlocks.at(-1);
  const finalNode = state.tailNodes.at(-1);
  const openFence =
    finalBlock?.type === "code" && findOpenCodeFence(tailSource);
  if (openFence && finalNode instanceof HTMLElement) {
    const pre = finalNode.querySelector("pre");
    if (pre) state.appendableTail = { type: "code", pre, fence: openFence };
  } else if (
    tailBlocks.length === 1 &&
    finalBlock?.type === "paragraph" &&
    isPlainAppendableText(tailSource) &&
    finalNode instanceof HTMLParagraphElement
  ) {
    state.appendableTail = { type: "plain-paragraph", paragraph: finalNode };
  }
  state.source = normalisedSource;
}
