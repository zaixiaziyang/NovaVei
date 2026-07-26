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

const renderedSources = new WeakMap<HTMLElement, string>();

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
  const lines = source.replace(/\r\n?/g, "\n").split("\n");
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
      const body: string[] = [];
      index += 1;
      while (
        index < lines.length &&
        !new RegExp(`^ {0,3}${marker}{${width},}\\s*$`).test(lines[index])
      ) {
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

type InlineKind = "code" | "link" | "strong" | "strike" | "em";
type InlineMatch = { kind: InlineKind; match: RegExpMatchArray };

function nextInlineMatch(source: string): InlineMatch | undefined {
  const candidates: InlineMatch[] = [];
  const add = (kind: InlineKind, match: RegExpMatchArray | null) => {
    if (match) candidates.push({ kind, match });
  };
  add("code", source.match(/`([^`\n]+)`/));
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
  if (renderedSources.get(target) === source) return;
  renderedSources.set(target, source);
  const fragment = document.createDocumentFragment();
  for (const block of parseMarkdown(source))
    fragment.appendChild(renderBlock(block));
  target.replaceChildren(fragment);
}
