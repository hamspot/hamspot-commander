import json
M=1e6
def ch(name,mhz,mode="FM",enc=False,control=False):
    return {"name":name,"hz":round(mhz*M),"mode":mode,"control":control,"encrypted":enc}
zones=[]
def Z(n,c): zones.append({"name":n,"channels":c})
Z("SKW Uintah Basin public safety",[
 ch("SKW Uintah Co Sheriff/Vernal PD 155.670",155.670),
 ch("SKW Vernal PD 155.640",155.640),
 ch("SKW Ft Duchesne Tribal PD 155.415",155.415),
 ch("SKW Uintah SAR 156.105",156.105),
 ch("SKW Uinta Basin EMS 155.055",155.055),
 ch("SKW Gold Cross EMS 460.550",460.550),
 ch("SKW Duchesne Law (Steamboat) 155.370",155.370),
 ch("SKW Duchesne Fire/EMS W (Tabby) 155.835",155.835),
 ch("SKW Duchesne SAR 151.010",151.010),
 ch("SKW Roosevelt PD/UHP 155.325",155.325),
 ch("SKW UHP Duchesne-Uintah link 453.225",453.225),
 ch("SKW UHP Bruin Peak 155.880",155.880),
 ch("SKW UHP Hoy Mtn 156.0825",156.0825),
 ch("SKW UCA P25 Roosevelt Hill CC 769.26875",769.26875,control=True),
])
Z("SKW Uintah Basin ham",[
 ch("SKW W7BAR Grizzly Ridge 147.040",147.040),
 ch("SKW W7BAR Blue Mtn 147.100",147.100),
 ch("SKW KG7DSO Tabby Mtn 147.340",147.340),
 ch("SKW W7BAR Uintah EOC 449.900",449.900),
 ch("SKW W7BYU Vernal 449.700 (status unknown)",449.700),
])
Z("A51 NTTR / Groom air (AM)",[
 ch("A51 Nellis Control West VHF 119.350",119.350,"AM"),
 ch("A51 Nellis Control West UHF 254.400",254.400,"AM"),
 ch("A51 Area 51 Ops Coffee Tree 123.225",123.225,"AM"),
 ch("A51 Dreamland MOA Control 126.150",126.150,"AM"),
 ch("A51 Dreamland MOA UHF 261.100",261.100,"AM"),
 ch("A51 Dreamland MOA 255.800",255.800,"AM"),
 ch("A51 Dreamland R-4807A Blond Girl 126.600",126.600,"AM"),
 ch("A51 Groom Lake MOA 361.300",361.300,"AM"),
 ch("A51 Range Control 259.400",259.400,"AM"),
 ch("A51 Blackjack VHF 123.550",123.550,"AM"),
 ch("A51 Blackjack Range UHF 377.800",377.800,"AM"),
 ch("A51 Blackjack 139.750",139.750,"AM"),
 ch("A51 Darkstar AWACS 391.800",391.800,"AM"),
 ch("A51 Coyote MOA Rachel area 379.100",379.100,"AM"),
 ch("A51 Salt Lake Center near Rachel 385.800",385.800,"AM"),
 ch("A51 NTTR Fire aircraft 120.525",120.525,"AM"),
 ch("A51 Janet Ops Gold Coast KLAS 118.700",118.700,"AM"),
 ch("A51 TTR Silverbow Tower 124.750",124.750,"AM"),
 ch("A51 TTR Silverbow Tower UHF 257.950",257.950,"AM"),
 ch("A51 TTR Base Ops Ridge Line 119.450",119.450,"AM"),
 ch("A51 Nellis AFB Tower 132.550",132.550,"AM"),
 ch("A51 Nellis Approach W 135.100",135.100,"AM"),
 ch("A51 Groom Lake VOR MCY 117.500",117.500,"AM"),
 ch("A51 NOAA WX Las Vegas 162.550",162.550),
])
Z("A51 Lincoln/Nye ground",[
 ch("A51 Lincoln Co SO Ch2 voice 154.860",154.860),
 ch("A51 Lincoln Co SAR mutual aid 155.160",155.160),
 ch("A51 Lincoln Co Fire/EMS 153.845",153.845),
 ch("A51 Alamo Fire 153.920",153.920),
 ch("A51 Nye Co SO Tonopah 154.830",154.830),
 ch("A51 NV LE mutual aid 155.655",155.655),
 ch("A51 Old Bus 856.525 (OLD)",856.525),
 ch("A51 NSRS P25 Mt Wilson CC 856.2125",856.2125,control=True),
 ch("A51 NSRS P25 Highland Pk CC 854.1375",854.1375,control=True),
 ch("A51 N7ARR Alamo 2m 147.240",147.240),
 ch("A51 W7AOR Pioche 2m 147.120",147.120),
 ch("A51 N7ARR Tonopah 2m 146.640",146.640),
])
Z("WATCH Radar / telemetry (waterfall only)",[
 ch("WATCH A51 CRIIS telemetry 1355.000",1355.0,"FM"),
 ch("WATCH A51 CRIIS telemetry 1366.000",1366.0,"FM"),
 ch("WATCH A51 RCS range CRIIS 1373.700",1373.7,"FM"),
 ch("WATCH A51 RCS range CRIIS 1375.700",1375.7,"FM"),
 ch("WATCH A51 ACTS telemetry 1819.000",1819.0,"FM"),
 ch("WATCH A51 ACTS telemetry 1845.000",1845.0,"FM"),
 ch("WATCH SSR interrogation 1030",1030.0,"AM"),
 ch("WATCH ADS-B / Mode S 1090",1090.0,"AM"),
 ch("WATCH GPS L2 1227.60",1227.60,"AM"),
 ch("WATCH GPS L1 1575.42",1575.42,"AM"),
 ch("WATCH GLONASS L1 1602.000",1602.0,"AM"),
 ch("WATCH Iridium 1626.000 (simplex ring)",1626.0,"AM"),
 ch("WATCH Hydrogen line 1420.405",1420.40575,"AM"),
])
Z("Distress / universal",[
 ch("Guard VHF 121.500",121.5,"AM"),
 ch("Guard UHF 243.000",243.0,"AM"),
 ch("ACARS 131.550",131.55,"AM"),
 ch("2m national simplex 146.520",146.52),
 ch("70cm national simplex 446.000",446.0),
 ch("FRS/GMRS ch1 462.5625",462.5625),
 ch("MURS ch1 151.820",151.82),
 ch("CB ch19 27.185",27.185,"AM"),
 ch("CB ch9 emergency 27.065",27.065,"AM"),
])
def seg(lo,hi,use="mixed",svc=None):
    s={"lo_hz":round(lo*M),"hi_hz":round(hi*M),"use":use}
    if svc: s["service"]=svc
    return s
def band(name,chip,lo,hi,home,mode,segs): return {"name":name,"chip":chip,"lo_hz":round(lo*M),"hi_hz":round(hi*M),"home_hz":round(home*M),"mode":mode,"segments":segs}
bsp={"version":1,"zones":[
 {"name":"Air","color":[200,170,90],"bands":[
   band("VHF Airband","air",118,137,121.5,"AM",[seg(118,137,"voice","aero")]),
   band("Mil UHF Air","milair",225,400,243,"AM",[seg(225,328.6,"voice","military"),seg(328.6,335.4,"digital","aero"),seg(335.4,400,"voice","military")]),
 ]},
 {"name":"Land mobile","color":[72,140,168],"bands":[
   band("VHF High","vhi",148,174,155.67,"FM",[seg(148,150.8,"mixed","military"),seg(150.8,162.0125,"mixed","government"),seg(162.0125,174,"mixed","military")]),
   band("UHF LMR","uhf",450,470,460.55,"FM",[seg(450,470,"mixed","government")]),
   band("800 trunking","t800",851,861,856.2125,"FM",[seg(851,861,"mixed","government")]),
 ]},
 {"name":"Ham / personal","color":[110,170,110],"bands":[
   band("CB","cb",26.965,27.405,27.185,"AM",[seg(26.965,27.405,"voice","cb")]),
   band("2 m","2m",144,148,146.52,"FM",[seg(144,144.1,"cw","ham"),seg(144.1,148,"voice","ham")]),
   band("70 cm","70cm",420,450,446,"FM",[seg(420,450,"mixed","ham")]),
   band("FRS/GMRS","gmrs",462.55,467.725,462.5625,"FM",[seg(462.55,467.725,"voice")]),
 ]},
 {"name":"Radar / L-band","color":[180,80,80],"bands":[
   band("DME/TACAN + SSR","dme",960,1215,1090,"AM",[seg(960,1029,"mixed","aero"),seg(1029,1031,"mixed","aero"),seg(1031,1089,"mixed","aero"),seg(1089,1091,"mixed","aero"),seg(1091,1215,"mixed","aero")]),
   band("L-band radar (ARSR) + CRIIS","lrad",1215,1400,1300,"AM",[seg(1215,1240,"mixed","government"),seg(1240,1350,"mixed","aero"),seg(1350,1390,"digital","military"),seg(1390,1400,"mixed")]),
   band("H-line radio astronomy","hline",1400,1427,1420.40575,"AM",[seg(1400,1427,"mixed")]),
   band("GNSS / Iridium / satcom (the '1.6 GHz')","l16",1525,1660.5,1626,"AM",[seg(1525,1559,"digital","marine"),seg(1559,1610,"digital","government"),seg(1610,1626.5,"mixed"),seg(1626.5,1660.5,"digital","marine")]),
   band("ACTS telemetry","acts",1780,1850,1819,"AM",[seg(1780,1850,"mixed","military")]),
 ]},
 {"name":"S-band radar / ISM","color":[200,110,60],"bands":[
   band("ISM 2.4 GHz / drones","ism24",2400,2483.5,2440,"FM",[seg(2400,2483.5,"mixed")]),
   band("S-band ASR / NEXRAD","srad",2700,3000,2800,"AM",[seg(2700,2900,"mixed","aero"),seg(2900,3000,"mixed","government")]),
   band("5.8 GHz FPV video","fpv",5725,5875,5800,"FM",[seg(5725,5875,"mixed")]),
 ]},
 {"name":"ISM UHF","color":[150,150,150],"bands":[
   band("433 ISM sensors","ism433",433.05,434.79,433.92,"FM",[seg(433.05,434.79,"mixed")]),
   band("915 ISM","ism915",902,928,915,"FM",[seg(902,928,"mixed")]),
 ]},
]}
base=json.load(open('examples/Example-Satellite-EME.HSCommander'))
base["bsp_name"]="ufo-hunters.json"; base["bsp"]=bsp
base["channels"]={"version":1,"zones":zones}
base["tune"].update({"zone_sel":0,"band_sel":0,"band_mem":{},"freq_hz":121500000,"mode":"AM","passband_hz":8000})
base["scan_zones"]=[0,2,3]
base["page"]="Channels"
json.dump(base,open('examples/stations/UFO-Hunters.HSCommander','w'),indent=2)
print(sum(len(z["channels"]) for z in zones), sum(len(b["segments"]) for z in bsp["zones"] for b in z["bands"]), sum(len(z["bands"]) for z in bsp["zones"]))
