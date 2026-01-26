/**
 * Dynamic width table rendering (inspired by ccusage)
 */

import Table from "cli-table3";
import pc from "picocolors";
import stringWidth from "string-width";

export type TableCellAlign = "left" | "right" | "center";
export type TableRow = (string | number | { content: string; hAlign?: TableCellAlign })[];

export interface TableOptions {
  head: string[];
  colAligns?: TableCellAlign[];
  style?: { head?: string[] };
  compactHead?: string[];
  compactColAligns?: TableCellAlign[];
  compactThreshold?: number;
}

export class ResponsiveTable {
  private head: string[];
  private rows: TableRow[] = [];
  private colAligns: TableCellAlign[];
  private style?: { head?: string[] };
  private compactHead?: string[];
  private compactColAligns?: TableCellAlign[];
  private compactThreshold: number;
  private compactMode = false;

  constructor(options: TableOptions) {
    this.head = options.head;
    this.colAligns = options.colAligns ?? Array.from({ length: this.head.length }, () => "left");
    this.style = options.style;
    this.compactHead = options.compactHead;
    this.compactColAligns = options.compactColAligns;
    this.compactThreshold = options.compactThreshold ?? 100;
  }

  push(row: TableRow): void {
    this.rows.push(row);
  }

  private filterRowToCompact(row: TableRow, compactIndices: number[]): TableRow {
    return compactIndices.map((index) => row[index] ?? "");
  }

  private getCurrentTableConfig(): { head: string[]; colAligns: TableCellAlign[] } {
    if (this.compactMode && this.compactHead && this.compactColAligns) {
      return { head: this.compactHead, colAligns: this.compactColAligns };
    }
    return { head: this.head, colAligns: this.colAligns };
  }

  private getCompactIndices(): number[] {
    if (!this.compactHead || !this.compactMode) {
      return Array.from({ length: this.head.length }, (_, i) => i);
    }
    return this.compactHead.map((compactHeader) => {
      const index = this.head.indexOf(compactHeader);
      return index < 0 ? 0 : index;
    });
  }

  toString(): string {
    const terminalWidth =
      Number.parseInt(process.env.COLUMNS ?? "", 10) || process.stdout.columns || 120;

    this.compactMode = terminalWidth < this.compactThreshold && this.compactHead != null;

    const { head, colAligns } = this.getCurrentTableConfig();
    const compactIndices = this.getCompactIndices();

    const processedRows = this.compactMode
      ? this.rows.map((row) => this.filterRowToCompact(row, compactIndices))
      : this.rows;

    const allRows = [
      head.map(String),
      ...processedRows.map((row) =>
        row.map((cell) => {
          if (typeof cell === "object" && cell != null && "content" in cell) {
            return String(cell.content);
          }
          return String(cell ?? "");
        })
      ),
    ];

    const contentWidths = head.map((_, colIndex) => {
      const maxLength = Math.max(...allRows.map((row) => stringWidth(String(row[colIndex] ?? ""))));
      return maxLength;
    });

    const numColumns = head.length;
    const tableOverhead = 3 * numColumns + 1;
    const availableWidth = terminalWidth - tableOverhead;

    const columnWidths = contentWidths.map((width, index) => {
      const align = colAligns[index];
      if (align === "right") {
        return Math.max(width + 3, 11);
      } else if (index === 1) {
        return Math.max(width + 2, 15);
      }
      return Math.max(width + 2, 10);
    });

    const totalRequiredWidth = columnWidths.reduce((sum, width) => sum + width, 0) + tableOverhead;

    let finalWidths = columnWidths;
    if (totalRequiredWidth > terminalWidth) {
      const scaleFactor = availableWidth / columnWidths.reduce((sum, width) => sum + width, 0);
      finalWidths = columnWidths.map((width, index) => {
        const align = colAligns[index];
        let adjustedWidth = Math.floor(width * scaleFactor);
        if (align === "right") {
          adjustedWidth = Math.max(adjustedWidth, 10);
        } else if (index === 0) {
          adjustedWidth = Math.max(adjustedWidth, 10);
        } else {
          adjustedWidth = Math.max(adjustedWidth, 8);
        }
        return adjustedWidth;
      });
    }

    const table = new Table({
      head,
      style: this.style,
      colAligns,
      colWidths: finalWidths,
      wordWrap: true,
      wrapOnWordBoundary: true,
    });

    for (const row of processedRows) {
      table.push(row as any);
    }

    return table.toString();
  }
}

export function formatNumber(num: number): string {
  return num.toLocaleString("en-US");
}

export function formatCurrency(amount: number): string {
  return `$${amount.toFixed(2)}`;
}

export function formatModelName(modelName: string): string {
  const claudeMatch = modelName.match(/claude-(\w+)-(.+)/);
  if (claudeMatch) {
    return `${claudeMatch[1]}-${claudeMatch[2]}`;
  }
  return modelName;
}

export function formatModelsMultiline(models: string[]): string {
  const unique = [...new Set(models.map(formatModelName))];
  return unique.sort().map((m) => `- ${m}`).join("\n");
}

export function createUsageTable(firstColumnName: string): ResponsiveTable {
  return new ResponsiveTable({
    head: [
      firstColumnName,
      "Models",
      "Input",
      "Output",
      "Cache Write",
      "Cache Read",
      "Total",
      "Cost",
    ],
    style: { head: ["cyan"] },
    colAligns: ["left", "left", "right", "right", "right", "right", "right", "right"],
    compactHead: [firstColumnName, "Models", "Input", "Output", "Cost"],
    compactColAligns: ["left", "left", "right", "right", "right"],
    compactThreshold: 100,
  });
}

export function formatUsageRow(
  firstCol: string,
  models: string[],
  input: number,
  output: number,
  cacheWrite: number,
  cacheRead: number,
  cost: number
): TableRow {
  const total = input + output + cacheWrite + cacheRead;
  return [
    firstCol,
    formatModelsMultiline(models),
    formatNumber(input),
    formatNumber(output),
    formatNumber(cacheWrite),
    formatNumber(cacheRead),
    formatNumber(total),
    formatCurrency(cost),
  ];
}

export function formatTotalsRow(
  input: number,
  output: number,
  cacheWrite: number,
  cacheRead: number,
  cost: number
): TableRow {
  const total = input + output + cacheWrite + cacheRead;
  return [
    pc.yellow("Total"),
    "",
    pc.yellow(formatNumber(input)),
    pc.yellow(formatNumber(output)),
    pc.yellow(formatNumber(cacheWrite)),
    pc.yellow(formatNumber(cacheRead)),
    pc.yellow(formatNumber(total)),
    pc.yellow(formatCurrency(cost)),
  ];
}
