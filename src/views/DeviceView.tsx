import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Smartphone, RefreshCw, Info, AlertTriangle, Play } from "lucide-react";
import { toast } from "../lib/toastStore";
import { runBusy } from "../lib/busyStore";

interface DeviceEntry {
  serial: string;
  state: string;
}

interface FlashPlan {
  partition: string;
  image: string;
  image_size: number;
  partition_size: number | null;
  dynamic: boolean;
  userspace_fastboot: boolean | null;
  bootloader_opt_in: boolean;
  refusal: string | null;
}

function fmt(bytes: number | null): string {
  if (bytes === null || bytes === undefined) return "—";
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024).toFixed(0)} KB`;
}

const INTERESTING = [
  "ro.product.model",
  "ro.product.device",
  "ro.build.version.release",
  "ro.build.version.sdk",
  "ro.build.fingerprint",
  "ro.boot.slot_suffix",
  "ro.boot.verifiedbootstate",
];

export function DeviceView() {
  const [adbList, setAdbList] = useState<DeviceEntry[]>([]);
  const [fbList, setFbList] = useState<DeviceEntry[]>([]);
  const [adbInfo, setAdbInfo] = useState<[string, string][]>([]);
  const [fbVars, setFbVars] = useState<[string, string][]>([]);
  const [fbFilter, setFbFilter] = useState("");
  const [scanned, setScanned] = useState(false);

  const [partition, setPartition] = useState("");
  const [image, setImage] = useState<string | null>(null);
  const [blOptIn, setBlOptIn] = useState(false);
  const [plan, setPlan] = useState<FlashPlan | null>(null);

  const scan = async () => {
    await runBusy("Scanning for devices", async () => {
      try {
        setAdbList(await invoke<DeviceEntry[]>("adb_devices"));
      } catch (e) {
        toast.error(`adb: ${e}`);
      }
      try {
        setFbList(await invoke<DeviceEntry[]>("fastboot_devices"));
      } catch (e) {
        toast.error(`fastboot: ${e}`);
      }
      setScanned(true);
      setAdbInfo([]);
      setFbVars([]);
      setPlan(null);
    });
  };

  const loadAdbInfo = async (serial: string) => {
    await runBusy("Reading device info", async () => {
      try {
        setAdbInfo(await invoke<[string, string][]>("adb_device_info", { serial }));
      } catch (e) {
        toast.error(`Info failed: ${e}`);
      }
    });
  };

  const loadFbVars = async () => {
    await runBusy("Reading bootloader variables", async () => {
      try {
        setFbVars(await invoke<[string, string][]>("fastboot_device_vars"));
      } catch (e) {
        toast.error(`getvar failed: ${e}`);
      }
    });
  };

  const reboot = async (transport: "adb" | "fastboot", serial: string, target: string) => {
    if (!confirm(`Reboot ${serial} into ${target}?`)) return;
    try {
      await runBusy(`Rebooting to ${target}`, () =>
        transport === "adb"
          ? invoke("adb_reboot", { serial, target })
          : invoke("fastboot_reboot", { serial, target }),
      );
      toast.success(`Rebooting into ${target}.`);
    } catch (e) {
      toast.error(`Reboot failed: ${e}`);
    }
  };

  const pickImage = async () => {
    try {
      const file = await open({
        multiple: false,
        filters: [{ name: "Image", extensions: ["img"] }],
      });
      if (file && typeof file === "string") {
        setImage(file);
        setPlan(null);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const planFlash = async () => {
    if (!partition.trim() || !image) {
      toast.error("Enter a partition and pick an image.");
      return;
    }
    try {
      await runBusy("Building flash plan", () =>
        invoke<FlashPlan>("flash_plan", { partition: partition.trim(), image, bootloaderOptIn: blOptIn }),
      ).then(setPlan);
    } catch (e) {
      toast.error(`Plan failed: ${e}`);
    }
  };

  const doFlash = async () => {
    if (!plan || plan.refusal) return;
    if (!confirm(`CONFIRM: write ${plan.image.split("/").pop()} (${fmt(plan.image_size)}) to partition "${plan.partition}"?\n\nA wrong image or interrupted write can leave a device that does not start.`)) return;
    try {
      await runBusy(`Flashing ${plan.partition}`, () =>
        invoke("flash_image", { partition: plan.partition, image: plan.image, bootloaderOptIn: blOptIn }),
      );
      toast.success(`${plan.partition} written.`);
      setPlan(null);
    } catch (e) {
      toast.error(`Flash failed: ${e}`);
    }
  };

  const adbDevice = adbList.find((d) => d.state === "device");
  const fbDevice = fbList[0];
  const slot = fbVars.find(([k]) => k === "current-slot")?.[1];
  const unlocked = fbVars.find(([k]) => k === "unlocked")?.[1];
  const userspace = fbVars.find(([k]) => k === "is-userspace")?.[1];

  return (
    <div className="flex-col">
      <div className="md-card">
        <div className="flex-row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
          <div className="md-card-title" style={{ margin: 0 }}>Connected Devices</div>
          <button className="primary flex-row" style={{ gap: 8 }} onClick={scan}>
            <RefreshCw size={16} />
            Scan
          </button>
        </div>
        {!scanned ? (
          <p style={{ marginTop: 12, fontSize: 13, color: "var(--md-sys-color-outline)" }}>
            Scans <code>adb devices</code> and <code>fastboot devices</code>. Everything here is read-only until you deliberately build a flash plan.
          </p>
        ) : (
          <div className="flex-row mt-4" style={{ flexWrap: "wrap", gap: 10 }}>
            {adbList.map((d) => (
              <span key={d.serial} className={`chip ${d.state === "device" ? "chip-ok" : "chip-warn"}`}>
                adb {d.serial} ({d.state})
              </span>
            ))}
            {fbList.map((d) => (
              <span key={d.serial} className="chip chip-ok">
                fastboot {d.serial}
              </span>
            ))}
            {adbList.length === 0 && fbList.length === 0 && (
              <span className="chip chip-warn">no devices found</span>
            )}
          </div>
        )}
      </div>

      {adbDevice && (
        <div className="md-card">
          <div className="flex-row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
            <div className="md-card-title" style={{ margin: 0 }}>Android (ADB) — {adbDevice.serial}</div>
            <div className="flex-row" style={{ flexWrap: "wrap", gap: 8 }}>
              <button className="secondary" onClick={() => loadAdbInfo(adbDevice.serial)}><Info size={15} style={{ marginRight: 6, verticalAlign: "middle" }} />Info</button>
              <button className="secondary" onClick={() => reboot("adb", adbDevice.serial, "bootloader")}>Bootloader</button>
              <button className="secondary" onClick={() => reboot("adb", adbDevice.serial, "fastbootd")}>Fastbootd</button>
              <button className="secondary" onClick={() => reboot("adb", adbDevice.serial, "recovery")}>Recovery</button>
              <button className="secondary" onClick={() => reboot("adb", adbDevice.serial, "download")}>Download</button>
            </div>
          </div>
          {adbInfo.length > 0 && (
            <div className="project-list flex-col" style={{ marginTop: 12, maxHeight: "30vh", overflowY: "auto" }}>
              {adbInfo.filter(([k]) => INTERESTING.includes(k)).map(([k, v]) => (
                <div key={k} className="flex-row" style={{ padding: "6px 12px", gap: 12 }}>
                  <span style={{ fontFamily: "monospace", fontSize: 12, color: "var(--md-sys-color-primary)", minWidth: 220 }}>{k}</span>
                  <span style={{ fontSize: 13, fontFamily: "monospace", wordBreak: "break-all" }}>{v}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {fbDevice && (
        <div className="md-card">
          <div className="flex-row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 12 }}>
            <div className="md-card-title" style={{ margin: 0 }}>Bootloader (fastboot) — {fbDevice.serial}</div>
            <button className="secondary" onClick={loadFbVars}><Info size={15} style={{ marginRight: 6, verticalAlign: "middle" }} />Read variables</button>
          </div>
          {fbVars.length > 0 && (
            <>
              <div className="flex-row" style={{ flexWrap: "wrap", gap: 10, marginTop: 12 }}>
                <span className="chip chip-ok">current slot: {slot ?? "?"}</span>
                <span className={`chip ${unlocked === "yes" ? "chip-ok" : "chip-warn"}`}>
                  bootloader {unlocked ?? "unknown"}
                </span>
                <span className={`chip ${userspace === "yes" ? "chip-ok" : ""}`} style={userspace !== "yes" ? { border: "1px solid var(--md-sys-color-outline)" } : undefined}>
                  {userspace === "yes" ? "fastbootd (userspace)" : "bootloader fastboot"}
                </span>
              </div>
              <input
                placeholder="filter variables (e.g. partition-size:system_a)..."
                value={fbFilter}
                onChange={(e) => setFbFilter(e.target.value)}
                style={{ marginTop: 12, width: "100%" }}
              />
              <FbVarList vars={fbVars} filter={fbFilter} />
            </>
          )}
        </div>
      )}

      <div className="md-card" style={{ border: "1px solid var(--md-sys-color-error)" }}>
        <div className="flex-row" style={{ color: "var(--md-sys-color-error)", fontWeight: 500 }}>
          <AlertTriangle size={20} />
          Flash an image (fastboot only) — experimental
        </div>
        <p style={{ marginTop: 12, fontSize: 13, lineHeight: 1.6 }}>
          Identity and radio partitions (<code>modemst*</code>, <code>persist</code>, <code>efs</code>, <code>nvdata</code>, …)
          are <strong>refused outright</strong> — nothing could put them back. Bootloader-chain partitions
          (<code>xbl</code>, <code>abl</code>, <code>tz</code>, …) need the explicit opt-in below. Dynamic
          partitions (system/vendor/product/odm) require <strong>fastbootd</strong>, not bootloader fastboot.
          Images larger than the target partition are refused.
        </p>
        <div className="flex-row mt-4" style={{ flexWrap: "wrap" }}>
          <div className="flex-col" style={{ gap: 6, flexGrow: 1, minWidth: 200 }}>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: 14 }}>Partition (e.g. boot, boot_a, vendor)</label>
            <input value={partition} onChange={(e) => { setPartition(e.target.value); setPlan(null); }} placeholder="boot" />
          </div>
          <div className="flex-col" style={{ gap: 6, flexGrow: 2, minWidth: 260 }}>
            <label style={{ color: "var(--md-sys-color-outline)", fontSize: 14 }}>Image</label>
            <div className="flex-row" style={{ gap: 8 }}>
              <input type="text" readOnly value={image || "No image selected"} />
              <button className="icon-btn" onClick={pickImage}><Play size={16} /></button>
            </div>
          </div>
        </div>
        <label className="flex-row" style={{ gap: 8, marginTop: 12, fontSize: 14 }}>
          <input type="checkbox" checked={blOptIn} onChange={(e) => { setBlOptIn(e.target.checked); setPlan(null); }} />
          I accept the risk of flashing bootloader-chain partitions
        </label>
        <div style={{ marginTop: 14 }}>
          <button className="secondary" onClick={planFlash}>Build flash plan</button>
        </div>
        {plan && (
          <div className="md-card probe-card" style={{ marginTop: 14 }}>
            <div className="flex-row" style={{ flexWrap: "wrap", gap: 10, alignItems: "center" }}>
              <Smartphone size={18} color={plan.refusal ? "var(--md-sys-color-error)" : "var(--md-sys-color-primary)"} />
              <span className={`chip ${plan.refusal ? "chip-warn" : "chip-ok"}`}>
                {plan.refusal ? "refused" : "ready"}
              </span>
              <span style={{ fontSize: 13 }}>
                {plan.partition}: {fmt(plan.image_size)} → partition {fmt(plan.partition_size)}
                {plan.dynamic ? " (dynamic, fastbootd)" : ""}
              </span>
            </div>
            {plan.refusal ? (
              <p style={{ marginTop: 10, fontSize: 13, color: "var(--md-sys-color-error)" }}>{plan.refusal}</p>
            ) : (
              <div style={{ marginTop: 10 }}>
                <button className="primary" style={{ backgroundColor: "#B3261E", color: "white" }} onClick={doFlash}>
                  Flash {plan.partition} now
                </button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function FbVarList({ vars, filter }: { vars: [string, string][]; filter: string }) {
  const f = filter.trim().toLowerCase();
  const visible = vars.filter(([k, v]) => !f || k.toLowerCase().includes(f) || v.toLowerCase().includes(f));
  return (
    <div className="project-list flex-col" style={{ marginTop: 12, maxHeight: "34vh", overflowY: "auto" }}>
      {visible.map(([k, v]) => (
        <div key={k} className="flex-row" style={{ padding: "5px 12px", gap: 12 }}>
          <span style={{ fontFamily: "monospace", fontSize: 12, color: "var(--md-sys-color-primary)", minWidth: 260 }}>{k}</span>
          <span style={{ fontSize: 12, fontFamily: "monospace", wordBreak: "break-all" }}>{v}</span>
        </div>
      ))}
      {visible.length === 0 && <p style={{ color: "var(--md-sys-color-outline)", fontSize: 13 }}>No variables match.</p>}
    </div>
  );
}
