"use client";

export interface TabItem<T extends string> {
  id: T;
  label: string;
}

export interface TabBarProps<T extends string> {
  tabs: TabItem<T>[];
  activeTab: T;
  onTabChange: (tab: T) => void;
}

export function TabBar<T extends string>({
  tabs,
  activeTab,
  onTabChange,
}: TabBarProps<T>) {
  return (
    <div
      className="inline-flex flex-row items-center rounded-[25px] border p-[6px]"
      style={{
        width: "fit-content",
        backgroundColor: "#1F1F20",
        borderColor: "#262627",
      }}
    >
      {tabs.map((tab) => (
        <button
          key={tab.id}
          onClick={() => onTabChange(tab.id)}
          className="flex items-center justify-center rounded-[25px] px-5 py-[10px] transition-colors"
          style={{
            backgroundColor: activeTab === tab.id ? "#2C2C2F" : "transparent",
          }}
        >
          <span
            className="text-lg font-semibold leading-none whitespace-nowrap"
            style={{
              color: activeTab === tab.id ? "#FFFFFF" : "rgba(255, 255, 255, 0.5)",
            }}
          >
            {tab.label}
          </span>
        </button>
      ))}
    </div>
  );
}
