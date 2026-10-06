/**
 * Strings shared by the activity pages (trades, notifications, audit): filters, pager, time
 * cells and the JSON viewer. Page-specific copy lives in `i18n/{trades,notifications,audit}.ts`.
 */
import { defineMessages } from "@/i18n";

const zh = {
  live: "实时更新",
  offline: "正在重连",
  symbol: {
    label: "标的",
    all: "全部标的",
    search: "搜索代码或名称",
    no_match: "没有匹配的标的",
    removed: "已移出投资范围",
    clear: "清除标的筛选",
    filter_by: "只看 {symbol}",
  },
  range: {
    label: "日期",
    from: "开始日期",
    to: "结束日期",
    all: "全部",
    today: "今日",
    d7: "近 7 天",
    d30: "近 30 天",
    ytd: "今年",
    invalid: "开始日期晚于结束日期",
  },
  clear_filters: "清除筛选",
  pager: {
    range: "第 {from}–{to} 条，共 {total} 条",
    none: "共 0 条",
    page: "{page} / {pages}",
    prev: "上一页",
    next: "下一页",
    size: "每页条数",
    size_option: "{n} 条/页",
  },
  json: {
    copy: "复制",
    copied: "已复制",
    empty: "无附加数据",
  },
  csv: {
    exporting: "正在导出…",
    exported: "已导出 {n} 条记录",
    truncated: "记录较多，仅导出最近 {n} 条",
    nothing: "当前筛选下没有可导出的记录",
  },
  summary: {
    truncated: "汇总基于最近 {n} 条记录（共 {total} 条）",
  },
};

const en: typeof zh = {
  live: "Live",
  offline: "Reconnecting",
  symbol: {
    label: "Symbol",
    all: "All symbols",
    search: "Search symbol or name",
    no_match: "No matching symbols",
    removed: "Removed from universe",
    clear: "Clear symbol filter",
    filter_by: "Only {symbol}",
  },
  range: {
    label: "Date",
    from: "From",
    to: "To",
    all: "All",
    today: "Today",
    d7: "7D",
    d30: "30D",
    ytd: "YTD",
    invalid: "The start date is after the end date",
  },
  clear_filters: "Clear filters",
  pager: {
    range: "{from}–{to} of {total}",
    none: "0 rows",
    page: "{page} / {pages}",
    prev: "Previous page",
    next: "Next page",
    size: "Rows per page",
    size_option: "{n} / page",
  },
  json: {
    copy: "Copy",
    copied: "Copied",
    empty: "No additional data",
  },
  csv: {
    exporting: "Exporting…",
    exported: "Exported {n} rows",
    truncated: "Large result: only the latest {n} rows were exported",
    nothing: "Nothing to export for the current filter",
  },
  summary: {
    truncated: "Summary covers the latest {n} of {total} rows",
  },
};

export const activityText = defineMessages(zh, en);
