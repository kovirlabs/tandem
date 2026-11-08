import { tDynamic, t } from "../i18n";
import { useApp, useCompanionName } from "../store/app";

export function NoticeBar() {
  const notice = useApp((s) => s.notice);
  const setNotice = useApp((s) => s.setNotice);
  const name = useCompanionName();
  if (!notice) return null;
  return (
    <div className="notice" role="status">
      <span>{tDynamic(notice.key, { name, error: notice.detail ?? "" })}</span>
      {notice.detail && notice.key !== "error.generic" && <span className="detail">{notice.detail}</span>}
      <button className="link" onClick={() => setNotice(null)}>
        {t("notice.dismiss")}
      </button>
    </div>
  );
}
