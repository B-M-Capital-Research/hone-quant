import { A } from "@solidjs/router";
import { Empty } from "@/components/ui";
import { locale } from "@/i18n";

export default function NotFound() {
  return (
    <Empty title={locale() === "zh" ? "页面不存在" : "Page not found"} icon="search">
      <A class="btn sm" href="/">
        {locale() === "zh" ? "返回总览" : "Back to overview"}
      </A>
    </Empty>
  );
}
