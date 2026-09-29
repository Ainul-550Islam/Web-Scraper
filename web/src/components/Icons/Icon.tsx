// Icon — registry: assembles one-file-per-glyph modules into the same lookup object.
// Consumers keep using `Icon[name]` / `keyof typeof Icon` unchanged.

import { IOverview } from "./svg/Overview";
import { IRuns } from "./svg/Runs";
import { IJobs } from "./svg/Jobs";
import { IWorkers } from "./svg/Workers";
import { IQueue } from "./svg/Queue";
import { IProxies } from "./svg/Proxies";
import { IPipeline } from "./svg/Pipeline";
import { IStorage } from "./svg/Storage";
import { IAlerts } from "./svg/Alerts";
import { IAudit } from "./svg/Audit";
import { IDiagnostics } from "./svg/Diagnostics";
import { IConfig } from "./svg/Config";
import { ISearch } from "./svg/Search";
import { IClose } from "./svg/Close";
import { IChevronRight } from "./svg/ChevronRight";
import { IChevronLeft } from "./svg/ChevronLeft";
import { ISun } from "./svg/Sun";
import { IMoon } from "./svg/Moon";
import { IMonitor } from "./svg/Monitor";
import { IRefresh } from "./svg/Refresh";
import { ICopy } from "./svg/Copy";
import { ICheck } from "./svg/Check";
import { IWarn } from "./svg/Warn";
import { IInfo } from "./svg/Info";
import { IWifi } from "./svg/Wifi";
import { IWifiOff } from "./svg/WifiOff";
import { ILock } from "./svg/Lock";
import { IPause } from "./svg/Pause";
import { IPlay } from "./svg/Play";
import { IMenu } from "./svg/Menu";
import { IDownload } from "./svg/Download";
import { IPlug } from "./svg/Plug";
import { IApi } from "./svg/Api";

export const Icon = {
  Overview: IOverview,
  Runs: IRuns,
  Jobs: IJobs,
  Workers: IWorkers,
  Queue: IQueue,
  Proxies: IProxies,
  Pipeline: IPipeline,
  Storage: IStorage,
  Alerts: IAlerts,
  Audit: IAudit,
  Diagnostics: IDiagnostics,
  Config: IConfig,
  Search: ISearch,
  Close: IClose,
  ChevronRight: IChevronRight,
  ChevronLeft: IChevronLeft,
  Sun: ISun,
  Moon: IMoon,
  Monitor: IMonitor,
  Refresh: IRefresh,
  Copy: ICopy,
  Check: ICheck,
  Warn: IWarn,
  Info: IInfo,
  Wifi: IWifi,
  WifiOff: IWifiOff,
  Lock: ILock,
  Pause: IPause,
  Play: IPlay,
  Menu: IMenu,
  Download: IDownload,
  Plug: IPlug,
  Api: IApi,
};
