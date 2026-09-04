{
  "Types": [
    {
      "ID": 0,
      "NativeName": "TimeSlot",
      "Type": "Table",
      "BuiltinType": true,
      "Children": [
        {
          "ID": 1,
          "NativeName": "start_time",
          "Type": "DateTime",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [],
          "NativeType": "LibertasDateTime"
        },
        {
          "ID": 2,
          "NativeName": "duration",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "TimeInterval",
              "Value": "true"
            },
            {
              "Name": "NumberType",
              "Value": "u32"
            },
            {
              "Name": "NumberStep",
              "Value": "1"
            }
          ],
          "NativeType": "u32"
        }
      ],
      "Attributes": [],
      "NativeType": ""
    },
    {
      "ID": 0,
      "NativeName": "ZoneDataProtocol",
      "Type": "Union",
      "BuiltinType": true,
      "Children": [
        {
          "ID": 1,
          "NativeName": "GetZoneInfo",
          "Type": "Nil",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Request",
              "Value": "true"
            },
            {
              "Name": "SubscriptionRequest",
              "Value": "true"
            },
            {
              "Name": "NextResponse",
              "Value": "ZoneInfo"
            },
            {
              "Name": "Cacheable",
              "Value": "true"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 2,
          "NativeName": "ZoneInfo",
          "Type": "Table",
          "BuiltinType": true,
          "Children": [
            {
              "ID": 3,
              "NativeName": "next_schedule",
              "Type": "TimeSlot",
              "BuiltinType": false,
              "Children": [],
              "Attributes": [],
              "NativeType": ""
            },
            {
              "ID": 4,
              "NativeName": "hold_off_periods",
              "Type": "List",
              "BuiltinType": true,
              "Children": [
                {
                  "ID": 5,
                  "NativeName": "",
                  "Type": "TimeSlot",
                  "BuiltinType": false,
                  "Children": [],
                  "Attributes": [],
                  "NativeType": ""
                }
              ],
              "Attributes": [],
              "NativeType": "Vec < TimeSlot >"
            }
          ],
          "Attributes": [
            {
              "Name": "Response",
              "Value": "true"
            },
            {
              "Name": "SubscriptionData",
              "Value": "true"
            },
            {
              "Name": "NextRequest",
              "Value": "UpdateHoldOff"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 6,
          "NativeName": "UpdateHoldOff",
          "Type": "Table",
          "BuiltinType": true,
          "Children": [
            {
              "ID": 7,
              "NativeName": "hold_off_periods",
              "Type": "List",
              "BuiltinType": true,
              "Children": [
                {
                  "ID": 8,
                  "NativeName": "",
                  "Type": "TimeSlot",
                  "BuiltinType": false,
                  "Children": [],
                  "Attributes": [],
                  "NativeType": ""
                }
              ],
              "Attributes": [
                {
                  "Name": "CopyFrom",
                  "Value": "$.hold_off_periods"
                }
              ],
              "NativeType": "Vec < TimeSlot >"
            }
          ],
          "Attributes": [
            {
              "Name": "Request",
              "Value": "true"
            },
            {
              "Name": "NextResponse",
              "Value": "ZoneInfo"
            }
          ],
          "NativeType": ""
        }
      ],
      "Attributes": [
        {
          "Name": "UnionExternallyTagged",
          "Value": "true"
        },
        {
          "Name": "LibertasProtocol",
          "Value": "true"
        }
      ],
      "NativeType": ""
    },
    {
      "ID": 0,
      "NativeName": "SprinklerHead",
      "Type": "Enumeration",
      "BuiltinType": true,
      "Children": [
        {
          "ID": 1,
          "NativeName": "SurfaceDrip",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "0"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 2,
          "NativeName": "Bubblers",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "1"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 3,
          "NativeName": "PopupSpray",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "2"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 4,
          "NativeName": "RotorsLowRate",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "3"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 5,
          "NativeName": "RotorsHighRate",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "4"
            }
          ],
          "NativeType": ""
        }
      ],
      "Attributes": [],
      "NativeType": ""
    },
    {
      "ID": 0,
      "NativeName": "SprinklerZone",
      "Type": "Table",
      "BuiltinType": true,
      "Children": [
        {
          "ID": 1,
          "NativeName": "zone_valve",
          "Type": "Device",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "TableHeader",
              "Value": "true"
            },
            {
              "Name": "Unique",
              "Value": "99"
            },
            {
              "Name": "DeviceType",
              "Value": "BQEBAUABBgI="
            }
          ],
          "NativeType": "LibertasDevice"
        },
        {
          "ID": 2,
          "NativeName": "field_capacity",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "NumberMin",
              "Value": "0"
            },
            {
              "Name": "NumberMax",
              "Value": "100"
            },
            {
              "Name": "NumberStep",
              "Value": "1"
            },
            {
              "Name": "Default",
              "Value": "100"
            },
            {
              "Name": "NumberType",
              "Value": "u8"
            }
          ],
          "NativeType": "u8"
        },
        {
          "ID": 3,
          "NativeName": "soil_type",
          "Type": "SoilType",
          "BuiltinType": false,
          "Children": [],
          "Attributes": [],
          "NativeType": ""
        },
        {
          "ID": 4,
          "NativeName": "plant_type",
          "Type": "PlantType",
          "BuiltinType": false,
          "Children": [],
          "Attributes": [
            {
              "Name": "Default",
              "Value": "0"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 5,
          "NativeName": "head",
          "Type": "SprinklerHead",
          "BuiltinType": false,
          "Children": [],
          "Attributes": [
            {
              "Name": "Default",
              "Value": "2"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 6,
          "NativeName": "zone_info",
          "Type": "Endpoint",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "ProtocolSchema",
              "Value": "ZoneDataProtocol"
            },
            {
              "Name": "EndpointServer",
              "Value": "true"
            },
            {
              "Name": "BaseObjects",
              "Value": "^.zone_valve"
            }
          ],
          "NativeType": "LibertasEndpoint"
        }
      ],
      "Attributes": [],
      "NativeType": ""
    },
    {
      "ID": 0,
      "NativeName": "PlantType",
      "Type": "Enumeration",
      "BuiltinType": true,
      "Children": [
        {
          "ID": 1,
          "NativeName": "Lawn",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "0"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 2,
          "NativeName": "FruitTrees",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "1"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 3,
          "NativeName": "Flowers",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "2"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 4,
          "NativeName": "Vegetables",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "3"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 5,
          "NativeName": "Citrus",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "4"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 6,
          "NativeName": "TreesBushes",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "5"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 7,
          "NativeName": "Xeriscape",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "6"
            }
          ],
          "NativeType": ""
        }
      ],
      "Attributes": [],
      "NativeType": ""
    },
    {
      "ID": 0,
      "NativeName": "SoilType",
      "Type": "Enumeration",
      "BuiltinType": true,
      "Children": [
        {
          "ID": 1,
          "NativeName": "Loam",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "0"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 2,
          "NativeName": "Clay",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "1"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 3,
          "NativeName": "ClayLoam",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "2"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 4,
          "NativeName": "SiltyClay",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "3"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 5,
          "NativeName": "SandyLoam",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "4"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 6,
          "NativeName": "LoamySand",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "5"
            }
          ],
          "NativeType": ""
        },
        {
          "ID": 7,
          "NativeName": "Sand",
          "Type": "Number",
          "BuiltinType": true,
          "Children": [],
          "Attributes": [
            {
              "Name": "Fixed",
              "Value": "6"
            }
          ],
          "NativeType": ""
        }
      ],
      "Attributes": [],
      "NativeType": ""
    }
  ],
  "Functions": [
    {
      "NativeName": "libertas_sprinkler",
      "Parameters": [
        {
          "ID": 0,
          "NativeName": "notification_list",
          "Type": "List",
          "BuiltinType": true,
          "Children": [
            {
              "ID": 1,
              "NativeName": "",
              "Type": "User",
              "BuiltinType": true,
              "Children": [],
              "Attributes": [
                {
                  "Name": "Unique",
                  "Value": "99"
                }
              ],
              "NativeType": "LibertasUser"
            }
          ],
          "Attributes": [
            {
              "Name": "Unordered",
              "Value": "true"
            }
          ],
          "NativeType": "Vec < LibertasUser >"
        },
        {
          "ID": 2,
          "NativeName": "zones",
          "Type": "List",
          "BuiltinType": true,
          "Children": [
            {
              "ID": 3,
              "NativeName": "",
              "Type": "SprinklerZone",
              "BuiltinType": false,
              "Children": [],
              "Attributes": [],
              "NativeType": ""
            }
          ],
          "Attributes": [
            {
              "Name": "SizeMin",
              "Value": "1"
            }
          ],
          "NativeType": "Vec < SprinklerZone >"
        }
      ],
      "Attributes": [
        {
          "Name": "StringResources",
          "Value": "A_RESOURCE_NAME,HOLD_OFF_UPDATED"
        }
      ]
    }
  ],
  "Attributes": [
    {
      "Name": "DefaultLocale",
      "Value": "en"
    }
  ]
}