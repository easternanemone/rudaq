![](_page_0_Picture_1.jpeg)

# **Motion Synergy API 3rd-party Software Libraries**

#### <span id="page-0-0"></span>**Table of Contents**

| Мо | tion S                       | ynergy API 3rd-party Software Libraries                | 1  |  |  |  |
|----|------------------------------|--------------------------------------------------------|----|--|--|--|
| 1  | Revision History             |                                                        |    |  |  |  |
| 2  | Introduction                 |                                                        |    |  |  |  |
|    | 2.1                          | 2.1 Purpose                                            |    |  |  |  |
|    | 2.2                          | Scope                                                  | 3  |  |  |  |
|    | 2.3                          | Audience                                               | 3  |  |  |  |
| 3  | 3rd-party Software Libraries |                                                        |    |  |  |  |
|    | 3.1                          | 1 Boost C++ Library                                    |    |  |  |  |
|    | 3.2                          | C++ Standard Library (Windows)                         |    |  |  |  |
|    | 3.3                          | Lua Scripting Library                                  |    |  |  |  |
|    | 3.4                          | NumericInput Control Library                           |    |  |  |  |
|    | 3.5                          | OxyPlot - Plotting Library for .NET                    | 8  |  |  |  |
|    | 3.6                          | ZLib Library                                           | 9  |  |  |  |
| 4  | Licer                        | 11                                                     |    |  |  |  |
|    | 4.1                          | Motion Synergy API License Text                        | 11 |  |  |  |
|    |                              | 4.1.1 Boost C++ Library License Text                   | 11 |  |  |  |
|    |                              | 4.1.2 C++ Standard Library (Windows) License Text      | 11 |  |  |  |
|    |                              | 4.1.3 Lua Scripting Library License Text               | 16 |  |  |  |
|    |                              | 4.1.4 NumericInput Control Library License Text        | 16 |  |  |  |
|    |                              | 4.1.5 OxyPlot - Plotting Library for .NET License Text | 16 |  |  |  |
|    |                              | 4.1.6 ZLib Library License Text                        | 17 |  |  |  |
|    | 4.2                          | 17                                                     |    |  |  |  |
|    |                              | 4.2.1 Standard MIT License                             | 17 |  |  |  |

![](_page_1_Picture_1.jpeg)

# <span id="page-1-0"></span>**1 Revision History**

| Issue | Date        | By            | Description                         |
|-------|-------------|---------------|-------------------------------------|
| A     | 21 OCT 2021 | Mark Gladding | Initial revision.                   |
| B     | 03 NOV 2021 | Mark Gladding | Added NumericInput Control Library. |

![](_page_2_Picture_1.jpeg)

# <span id="page-2-0"></span>**2 Introduction**

### <span id="page-2-1"></span>**2.1 Purpose**

This document details the 3rd-party Software Libraries and their associated licenses used by the MotionSynergyGUI and MotionSynergyAPI dll.

### <span id="page-2-2"></span>**2.2 Scope**

This document covers the following information related to 3rd-party libraries:

- Identification of 3rd-party libraries.
- Defines the functional and performance requirements for each 3rd-party library (covered by the *Intended Use* and *Selection Justification* sections for each library).
- Selection and validation strategy for each 3rd-party library.
- Details the license(s) for each 3rd-party library.
- Documents the title, version, manufacturer for each 3rd-party library.
- Assigns a unique designator for each 3rd-party library.
- Software (dependencies) necessary to support the proper operation of the 3rd-party library.

### <span id="page-2-3"></span>**2.3 Audience**

This document provides a reference of selected 3rd-party libraries and a record of the selection criteria used for each 3rd-party library.

The intended audience for this document includes:

- Software team members charged with the design and implementation of the software.
- Software and system testers charged with verifying the correct operation of the software.
- Future software developers charged with maintenance of the software.

![](_page_3_Picture_1.jpeg)

# <span id="page-3-0"></span>**3 3rd-party Software Libraries**

### <span id="page-3-1"></span>**3.1 Boost C++ Library**

**ID**

IMP-SOUP-BOOST

**Title**

Boost C++ Library

**Version**

1.66.0

#### **Manufacturer**

Not Applicable (Open Source)

#### **Description**

Boost is a set of high-quality, free, peer-reviewed, open-source cross-platform C++ libraries. It works well with the C++ Standard Library. Use of Boost reduces initial development costs, results in fewer bugs, reduces reinvention-of-the-wheel, and cuts long-term maintenance costs.

### **URL**

<https://www.boost.org/>

#### **Intended Use**

Boost has two primary purposes:

- 1. Provide an abstraction layer which hides operating system specifics. This allows crossplatform (e.g. Linux and Windows) applications to be developed with minimal effort.
- 2. Provides a set of general purpose libraries which reduce the amount of code needed to develop a specific application. Some commonly used libraries include asio (network and serial communications, timers), regex (regular expressions), property trees (configuration file support), etc.

#### **Selection Justification**

Following is the list of attributes used in the selection of this library:

- It is a mature library with wide adoption across the software industry.
- It has active development and patches.
- Royalty-free license conditions permitting use in a closed source, commercial product.
- Cross-platform support for both Linux and Windows.
- Minimal or no dependencies on other libraries.
- Functions correctly within the CPU, memory and persistent storage resource constraints of the Central Controller software runtime environment.

### **License**

[Boost Software License, Version 1.0](https://www.boost.org/LICENSE_1_0.txt)

#### **Dependent Units**

#### **Dependencies**

![](_page_4_Picture_1.jpeg)

• IMP-SOUP-CPPLIBLINUX or IMP-SOUP-CPPLIBWIN (depending on the target platform)

#### **Limitations**

No known issues impacting Dependent Units.

#### **Validation Strategy**

Validated as part of the unit/integration tests of dependent Software Units in the required context.

# <span id="page-4-0"></span>**3.2 C++ Standard Library (Windows)**

### **ID**

IMP-SOUP-CPPLIBWIN

#### **Title**

C++ Standard Library (Windows)

### **Version**

Microsoft Visual Studio 2019 v16.11.6 Microsoft Visual C++ Toolset v142

### **Namespace**

std

#### **Manufacturer**

Microsoft

#### **Description**

Provides an implementation of the standard libraries associated with the C++ language. These libraries include:

- C++ Standard Library
- C Standard Library
- C Maths Library

#### **URL**

<https://visualstudio.microsoft.com/>

#### **Intended Use**

The C++ standard library has two primary purposes:

- 1. Provide an abstraction layer which hides operating system specifics. This allows crossplatform (e.g. Linux and Windows) applications to be developed with minimal effort.
- 2. Provides a set of general purpose libraries which reduce the amount of code needed to develop a specific application. Some commonly used libraries include containers, shared pointers, threading, etc.

#### **Selection Justification**

Included as part of the Microsoft Visual Studio C++ toolset, a high quality, continuously maintained product used industry-wide for development on the Microsoft Windows platform.

#### **License**

[Microsoft Visual Studio License](https://visualstudio.microsoft.com/license-terms/mlt031619/)

![](_page_5_Picture_1.jpeg)

#### **Dependent Units**

#### **Dependencies**

None.

#### **Limitations**

No known issues impacting Dependent Units.

### **Validation Strategy**

Validated as part of the unit/integration tests of dependent Software Units in the required context.

### <span id="page-5-0"></span>**3.3 Lua Scripting Library**

#### **ID**

IMP-SOUP-LUA

### **Title**

Lua Scripting Library

#### **Version**

5.3.4

#### **Manufacturer**

PUC-Rio

#### **Description**

Lua is a powerful, efficient, lightweight, embeddable scripting language. Lua is dynamically typed, runs by interpreting bytecode with a register-based virtual machine, and has automatic memory management with incremental garbage collection, making it ideal for configuration, scripting, and rapid prototyping. Several benchmarks show Lua as the fastest language in the realm of interpreted scripting languages. Lua is fast not only in fine-tuned benchmark programs, but in real life too.

#### **URL**

<https://www.lua.org/>

### **Intended Use**

Lua provides a scripting language for creating workflows to coordinate the control of multiple hardware devices. Typical workflows include initialising hardware, performing a power on self test, running an assay, performing a maintenance function such as emptying waste, etc.Lua scripting is not intended to perform hard realtime operations (i.e. requiring sub-50ms timing precision) which involve coordination of two or more hardware devices due to latency and jitter between the script command execution and the hardware control.

### **Selection Justification**

Lua has been used in many industrial applications (e.g., Adobe's Photoshop Lightroom), with an emphasis on embedded systems (e.g., the Ginga middleware for digital TV in Brazil) and games (e.g., World of Warcraft and Angry Birds). Lua is currently the leading scripting language in games. Lua has a solid reference manual and there are several books about it. Several versions of Lua have been released and used in real applications since its creation in 1993.

![](_page_6_Picture_1.jpeg)

Lua was selected as it can be run on both the Central Controller and the resource-constrained embedded environment of the Subsystem Controller. Currently Lua is only used on the Central Controller.

#### **License**

[MIT License](https://www.lua.org/license.html)

#### **Dependent Units**

#### **Dependencies**

• IMP-SOUP-CPPLIBLINUX or IMP-SOUP-CPPLIBWIN (depending on the target platform)

#### **Limitations**

No known issues impacting Dependent Units.

#### **Validation Strategy**

Validated as part of the unit/integration tests of dependent Software Units in the required context.

### <span id="page-6-0"></span>**3.4 NumericInput Control Library**

#### **ID**

IMP-SOUP-NUMERIC-INPUT

#### **Title**

NumericInput Control Library

#### **Version**

0.5.5

#### **Namespace**

Gu.Wpf.NumericInput

### **Manufacturer**

Johan Larsson

#### **Description**

WPF TextBox for numeric input. Includes controls DoubleBox, IntBox, DecimalBox, FloatBox and ShortBox.

### **URL**

<https://github.com/GuOrg/Gu.Wpf.NumericInput>

#### **Intended Use**

Provides controls for numeric input via the MotionSynergyGUI.

#### **Selection Justification**

Following is the list of attributes used in the selection of this library:

- It is a mature library with wide adoption across the software industry.
- It has active development and patches.
- Royalty-free license conditions permitting use in a closed source, commercial product.
- Cross-platform support for Linux, QNX and Windows.

![](_page_7_Picture_1.jpeg)

- Minimal or no dependencies on other libraries.
- Functions correctly within the CPU, memory and persistent storage resource constraints of the MotionSynergyGUI software runtime environment.

#### **License**

[MIT License](https://github.com/GuOrg/Gu.Wpf.NumericInput/blob/master/LICENSE)

#### **Dependent Units**

• MotionSynergyGUI

#### **Dependencies**

None

#### **Limitations**

No known issues impacting Dependent Units.

#### **Validation Strategy**

Validated as part of the unit/integration tests of dependent Software Units in the required context.

### <span id="page-7-0"></span>**3.5 OxyPlot - Plotting Library for .NET**

#### **ID**

IMP-SOUP-OXYPLOT

### **Title**

OxyPlot - Plotting Library for .NET

#### **Version**

2.1.2

#### **Namespace**

OxyPlot.Core, OxyPlot.Wpf, OxyPlot.Wpf.Shared

### **Manufacturer**

OxyPlot contributors

# **Description**

OxyPlot is a cross-platform plotting library for .NET. A number of support packages are available to support a specific UI framework.

#### **URL**

<https://oxyplot.github.io/>

#### **Intended Use**

Provides plotting capabilities in the MotionSynergyGUI.

#### **Selection Justification**

Following is the list of attributes used in the selection of this library:

- It is a mature library with wide adoption across the software industry.
- It has active development and patches.
- Royalty-free license conditions permitting use in a closed source, commercial product.

![](_page_8_Picture_1.jpeg)

- Cross-platform support for Linux and Windows.
- Minimal or no dependencies on other libraries.
- Functions correctly within the CPU, memory and persistent storage resource constraints of the MotionSynergyGUI software runtime environment.

#### **License**

#### [MIT License](https://github.com/oxyplot/oxyplot/blob/develop/LICENSE)

#### **Dependent Units**

• MotionSynergyGUI

**Dependencies** OxyPlot.Core package implements the core plotting related APIs. However, to use OxyPlot with a UI framework, support packages for the UI framework are required. For example, OxyPlot.Wpf and OxyPlot.Wpf.Shared packages are required for OxyPlot to work with WPF UI framework.

#### **Limitations**

No known issues impacting Dependent Units.

#### **Validation Strategy**

Validated as part of the unit/integration tests of dependent Software Units in the required context.

## <span id="page-8-0"></span>**3.6 ZLib Library**

#### **ID**

IMP-SOUP-ZLIB

#### **Title**

ZLib Library

### **Version**

1.2.11

#### **Manufacturer**

Jean-loup Gailly and Mark Adler

#### **Description**

zlib is designed to be a free, general-purpose, legally unencumbered -- that is, not covered by any patents -- lossless data-compression library for use on virtually any computer hardware and operating system. The zlib data format is itself portable across platforms.

### **URL**

<https://www.zlib.net/>

#### **Intended Use**

zlib provides the compression algorithms used when creating zip archives of log files.

#### **Selection Justification**

Following is the list of attributes used in the selection of this library:

• It is a mature library with wide adoption across the software industry.

![](_page_9_Picture_1.jpeg)

- It has active development and patches.
- Royalty-free license conditions permitting use in a closed source, commercial product.
- Cross-platform support for both Linux and Windows.
- Minimal or no dependencies on other libraries.
- Functions correctly within the CPU, memory and persistent storage resource constraints of the Central Controller software runtime environment.

#### **License**

[ZLib license](https://www.zlib.net/zlib_license.html)

#### **Dependent Units**

# **Dependencies**

None.

#### **Limitations**

No known issues impacting Dependent Units.

#### **Validation Strategy**

Validated as part of the unit/integration tests of dependent Software Units in the required context.

![](_page_10_Picture_1.jpeg)

# <span id="page-10-0"></span>**4 License Text**

### <span id="page-10-1"></span>**4.1 Motion Synergy API License Text**

#### <span id="page-10-2"></span>**4.1.1 Boost C++ Library License Text**

Boost Software License - Version 1.0 - August 17th, 2003

Permission is hereby granted, free of charge, to any person or organization obtaining a copy of the software and accompanying documentation covered by this license (the "Software") to use, reproduce, display, distribute, execute, and transmit the Software, and to prepare derivative works of the Software, and to permit third-parties to whom the Software is furnished to do so, all subject to the following:

The copyright notices in the Software and this entire statement, including the above license grant, this restriction and the following disclaimer, must be included in all copies of the Software, in whole or in part, and all derivative works of the Software, unless such copies or derivative works are solely in the form of machine-executable object code generated by a source language processor.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, TITLE AND NON-INFRINGEMENT. IN NO EVENT SHALL THE COPYRIGHT HOLDERS OR ANYONE DISTRIBUTING THE SOFTWARE BE LIABLE FOR ANY DAMAGES OR OTHER LIABILITY, WHETHER IN CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

#### <span id="page-10-3"></span>**4.1.2 C++ Standard Library (Windows) License Text**

and Canada, call (800) MICROSOFT or see aka.ms/nareturns.

MICROSOFT SOFTWARE LICENSE TERMS

MICROSOFT VISUAL STUDIO ENTERPRISE 2019, VISUAL STUDIO PROFESSIONAL 2019, VISUAL STUDIO TEST PROFESSIONAL 2019 AND TRIAL EDITION

\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_

These license terms are an agreement between you and Microsoft Corporation (or based on where you live, one of its affiliates). They apply to the software named above. The terms also apply to any Microsoft

services and updates for the software, except to the extent those have different terms. BY USING THE SOFTWARE, YOU ACCEPT THESE TERMS. IF YOU DO NOT ACCEPT THEM, DO NOT USE THE SOFTWARE. INSTEAD, RETURN IT TO THE RETAILER FOR A REFUND OR CREDIT. If you cannot obtain a refund there, contact

Microsoft about Microsoft's refund policies. See www.microsoft.com/worldwide. In the United States

\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_\_

TRIAL EDITION USE RIGHTS. If you have not acquired a valid full-use license, the software is a trial edition, and this Section applies to your use of the trial edition.

A. GENERAL. You may use any number of copies of the trial edition on your devices. You may only use the trial edition for internal evaluation purposes, and only during the trial period. You may not distribute or deploy any applications you make with the trial edition to a production environment. You may run load tests of up to 250 virtual users during the trial period.

B. TRIAL PERIOD AND CONVERSION. The trial period lasts for 30 days after you install the trial edition, plus any permitted extension period. After the expiration of the trial period, the trial edition will stop running. You may extend the trial period an additional 90 days if you sign in to the software. You may not be able to access data used with the trial edition after it stops running. You may convert your trial rights at any time to the full-use rights described below by acquiring a valid full-use license.

C. DISCLAIMER OF WARRANTY. THE TRIAL EDITION IS LICENSED "AS-ISâ€•. YOU BEAR THE RISK OF USING IT.

![](_page_11_Picture_1.jpeg)

MICROSOFT GIVES NO EXPRESS WARRANTIES, GUARANTEES OR CONDITIONS. TO THE EXTENT PERMITTED UNDER YOUR LOCAL LAWS, MICROSOFT EXCLUDES THE IMPLIED WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NON-INFRINGEMENT.

FOR AUSTRALIA – YOU HAVE STATUTORY GUARANTEES UNDER THE AUSTRALIAN CONSUMER LAW AND NOTHING IN THESE TERMS IS INTENDED TO AFFECT THOSE RIGHTS.

- D. SUPPORT. Because the trial edition is "as isâ€•, we may not provide support services for it.
- E. LIMITATIONS ON DAMAGES. YOU CAN RECOVER FROM MICROSOFT AND ITS SUPPLIERS ONLY DIRECT DAMAGES UP TO U.S. \$5.00. YOU CANNOT RECOVER ANY OTHER DAMAGES, INCLUDING CONSEQUENTIAL, LOST PROFITS, SPECIAL, INDIRECT OR INCIDENTAL DAMAGES.

This limitation applies to (a) anything related to the trial version, services, content (including code) on third party Internet sites, or third party programs; and (b) claims for breach of contract, breach of warranty, guarantee or condition, strict liability, negligence, or other tort to the extent permitted by applicable law.

It also applies even if Microsoft knew or should have known about the possibility of the damages. The above limitation or exclusion may not apply to you because your country may not allow the exclusion or limitation of incidental, consequential or other damages.

FULL-USE LICENSE TERMS FOR THE SOFTWARE: When you acquire a valid license and either enter a product key or sign in to the software, the terms below apply. You may not share your product key or access credentials.

- 1. OVERVIEW.
- a. Software. The software includes development tools, applications, and documentation.
- b. License Model. The software is licensed on a per user basis.
- 2. USE RIGHTS.
- a. General. One user may use copies of the software on your devices to develop and test applications. This includes using copies of the software on your own internal servers that remain fully dedicated to your own use. You may not, however, separate the components of the software (except as otherwise stated in this agreement) and run those in a production environment, or on third party devices, or for any purpose other than developing and testing your applications. Running the software on Microsoft Azure may require separate online usage fees.
- b. Workloads. These license terms apply to your use of the workloads made available to you within the software, except to the extent a workload or a workload component comes with different license terms and support policies.
- c. Backup copy. You may make one backup copy of the software, for reinstalling the software.
- d. Online Services in the Software. Some features of the software make use of online services to provide you with information about updates to the software or extensions, or to enable you to retrieve content, collaborate with others, or otherwise supplement your development experience. As used throughout this agreement, the term "softwareâ€• includes these online service features.
- e. Demo Use. The use rights permitted above include using the software to demonstrate your applications.
- 3. TERMS FOR SPECIFIC COMPONENTS. a. Utilities. The software contains items on the Utilities List at https://aka.ms/vs/16/utilities. You may copy and install those items onto your devices to debug and deploy your applications and databases you developed with the software. The Utilities are designed for temporary use. Microsoft may not be able to patch or update Utilities separately from the rest of the software. Some Utilities by their nature may make it possible for others to access the devices on which the Utilities are installed. You should delete all Utilities you have installed after you finish debugging or deploying your applications and databases. Microsoft is not responsible for any third party use or access of devices, or of the applications or databases on devices, on which Utilities have been installed.
- b. Build Devices and Visual Studio Build Tools. You may copy and install files from the software or from Visual Studio Build Tools onto your build devices, including physical devices and virtual machines or containers on those machines, whether on-premises or remote machines that are owned by you, hosted on Microsoft Azure for you, or dedicated solely to your use (collectively, "Build Devicesâ€•). You and others in your organization may use these files on your Build Devices solely to compile, build, and verify applications developed by using the software, or run quality or performance tests of those applications as part of the build process.

![](_page_12_Picture_1.jpeg)

- c. Font Components. While the software is running, you may use its fonts to display and print content. You may only: (i) embed fonts in content as permitted by the embedding restrictions in the fonts; and (ii) temporarily download them to a printer or other output device to help print content.
- d. Licenses for Other Components.
- · Microsoft Platforms. The software may include components from Microsoft Windows, Microsoft Windows Server, Microsoft SQL Server, Microsoft Exchange, Microsoft Office, or Microsoft SharePoint. These components are governed by separate agreements and their own product support policies, as described in the Microsoft "Licensesâ€• folder accompanying the software, except that, if separate license terms for those components are included in the associated installation directly, those license terms control. · Third Party Components. The software may include third party components with separate legal notices or governed by other agreements, as may be described in the ThirdPartyNotices file(s) accompanying the software.
- e. Package Managers. The software includes package managers, like NuGet, that give you the option to download other Microsoft and third party software packages to use with your applications. Those packages are under their own licenses, and not these license terms. Microsoft does not distribute, license or provide any warranties for any of the third party packages.
- 4. DISTRIBUTABLE CODE. The software contains code that you are permitted to distribute in applications you develop as described in this Section. For purposes of this Section 4, the term "distributionâ€• also means deployment of your applications for third parties to access over the Internet.
- a. Right to Use and Distribute. The code and text files listed below are "Distributable Codeâ€•.
- · Distributable List. You may copy and distribute the object code form of code listed on the Distributable List located at https://aka.ms/vs/16/redistribution.
- · Sample Code, Templates, and Styles. You may copy, modify, and distribute the source and object code form of code marked as "sampleâ€•, "templateâ€•, "simple stylesâ€•, and "sketch stylesâ€•.
- · Third Party Distribution. You may permit distributors of your applications to copy and distribute the Distributable Code as part of those applications.
- b. Distribution Requirements. For any Distributable Code you distribute, you must:
- · add significant primary functionality to it in your applications;
- · require distributors and external end users to agree to terms that protect the Distributable Code at least as much as this agreement; and
- · indemnify, defend, and hold harmless Microsoft from any claims, including attorneys' fees, related to the distribution or use of your applications, except to the extent that any claim is based solely on the Distributable Code.
- c. Distribution Restrictions. You may not:
- · use Microsoft's trademarks in your applications' names or in a way that suggests your applications come from or are endorsed by Microsoft; or
- · modify or distribute the source code of any Distributable Code so that any part of it becomes subject to an Excluded License. An Excluded License is one that requires, as a condition of use, modification or distribution of code, that (i) it be disclosed or distributed in source code form; or (ii) others have the right to modify it.
- 5. DEVELOPING EXTENSIONS.
- a. Limits on Extensions. You may not develop or enable others to develop extensions for the software (or any other component of the Visual Studio family of products) which circumvent the technical limitations implemented in the software. If Microsoft technically limits or disables extensibility for the software, you may not extend the software by, among other things, loading or injecting into the software any non-Microsoft add-ins, macros, or packages; modifying the software registry settings; or adding features or functionality equivalent to that found in the Visual Studio family of products.
- b. No Degrading the Software. If you develop an extension for the software (or any other component of the Visual Studio family of products), you must test the installation, uninstallation, and operation of your extension to ensure that such processes do not disable any features or adversely affect the functionality of the software (or such component) or of any previous version or edition of thereof.
- 6. DATA.

![](_page_13_Picture_1.jpeg)

- a. Data Collection. The software may collect information about you and your use of the software, and send that to Microsoft. Microsoft may use this information to provide services and improve our products and services. You may opt out of many of these scenarios, but not all, as described in the software documentation. There are also some features in the software that may enable you and Microsoft to collect data from users of your applications. If you use these features, you must comply with applicable law, including providing appropriate notices to users of your applications together with Microsoft's privacy statement. Our privacy statement is located at https://go.microsoft.com/fwlink/?LinkID=824704. You can learn more about data collection and its use from the software documentation and our privacy statement. Your use of the software operates as your consent to these practices.
- b. Processing of Personal Data. To the extent Microsoft is a processor or subprocessor of personal data in connection with the software, Microsoft makes the commitments in the European Union General Data Protection Regulation Terms of the Online Services Terms to all customers effective May 25, 2018, at https://docs.microsoft.com/en-us/legal/gdpr.
- 7. SCOPE OF LICENSE. The software is licensed, not sold. These license terms only give you some rights to use the software. Microsoft reserves all other rights. Unless applicable law gives you more rights despite this limitation, you may use the software only as expressly permitted in these license terms. In doing so, you must comply with any technical limitations in the software that only allow you to use it in certain ways. In addition, you may not:
- · work around any technical limitations in the software;
- · reverse engineer, decompile or disassemble the software, or otherwise attempt to derive the source code for the software, except and to the extent required by third party licensing terms governing use of certain open source components that may be included in the software;
- · remove, minimize, block, or modify any notices of Microsoft or its suppliers in the software;
- · use the software in any way that is against the law;
- · share, publish, rent, or lease the software; or
- · provide the software as a stand-alone offering or combine it with any of your applications for others to use.
- 8. NOT FOR RESALE SOFTWARE. You may not sell the software if it is marked as "NFRâ€• or "Not for Resaleâ€•.
- 9. PREVIOUS VERSIONS OR OTHER EDITIONS. These license terms do not supersede your right to use validly licensed previous versions or other editions of the software. You may use the software and those previous versions or other editions of the software concurrently.
- 10. PROOF OF LICENSE. If you acquired the software on a disc or other media, your proof of license is the Microsoft certificate of authenticity label, the accompanying product key, and your receipt. If you purchased an online copy of the software, your proof of license is the Microsoft product key you received with your purchase and your receipt and/or being able to access the software service through your Microsoft account. To identify genuine Microsoft software, see www.howtotell.com.
- 11. TRANSFER TO A THIRD PARTY. If you are a valid licensee of the software, you may transfer it and this agreement directly to another party. Before the transfer, that party must agree that these license terms apply to the transfer and use of the software. The transfer must include the software, this agreement, the genuine Microsoft product key, and (if applicable) the Proof of License label. The transferor must uninstall all copies of the software after transferring it from the device. The transferor may not retain any copies of the genuine Microsoft product key to be transferred, and may only retain copies of the software if otherwise licensed to do so. If you have acquired a non-perpetual license to use the software or if the software is marked Not for Resale you may not transfer the software or the software license agreement to another party.
- 12. EXPORT RESTRICTIONS. You must comply with all domestic and international export laws and regulations that apply to the software, which include restrictions on destinations, end users, and end use. For further information on export restrictions, visit www.microsoft.com/exporting. 13. SUPPORT. Microsoft provides support for the software as described at https://support.microsoft.com.
- 14. ENTIRE AGREEMENT. These license terms (including the warranty below), and the terms for supplements, updates, Internet-based services, and support services, are the entire agreement for the software and support services.
- 15. APPLICABLE LAW. If you acquired the software in the United States, Washington State law applies to interpretation of and claims for breach of this agreement, and the laws of the state where you live

#### Motion Synergy API 3rd-party Software Libraries

![](_page_14_Picture_1.jpeg)

apply to all other claims. If you acquire the software in any other country, its laws apply.

- 16. CONSUMER RIGHTS; REGIONAL VARIATIONS. These license terms describe certain legal rights. You may have other rights, including consumer rights, under the laws of your state or country. You may also have rights with respect to the party from which you acquired the software. This agreement does not change those other rights if the laws of your state or country do not permit it to do so. For example, if you acquired the software in one of the below regions, or if mandatory country law applies, then the following provisions apply to you:
- a) Australia. References to "Limited Warrantyâ€• are references to the express warranty provided by Microsoft. This warranty is given in addition to other rights and remedies you may have under law, including your rights and remedies in accordance with the statutory guarantees in the Australian Consumer Law.

In this section, "goodsâ€• refers to the software for which Microsoft provides the express warranty. Our goods come with guarantees that cannot be excluded under the Australian Consumer Law. You are entitled to a replacement or refund for a major failure and compensation for any other reasonably foreseeable loss or damage. You are also entitled to have the goods repaired or replaced if the goods fail to be of acceptable quality and the failure does not amount to a major failure.

- b) Canada. You may stop receiving updates on your device by turning off Internet access. If and when you re-connect to the Internet, the software will resume checking for and installing updates.
- c) Germany and Austria.
- (i) Warranty. The properly licensed software will perform substantially as described in any Microsoft materials that accompany the software. However, Microsoft gives no contractual guarantee in relation to the software.
- (ii) Limitation of Liability. In case of intentional conduct, gross negligence, claims based on the Product Liability Act, as well as, in the case of death or personal or physical injury, Microsoft is liable according to the statutory law.

Subject to the preceding sentence (ii), Microsoft will only be liable for slight negligence if Microsoft is in breach of such material contractual obligations, the fulfillment of which facilitate the due performance of this agreement, the breach of which would endanger the purpose of this agreement and the compliance with which a party may constantly trust in (so-called "cardinal obligations"). In other cases of slight negligence, Microsoft will not be liable for slight negligence.

\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*\*

#### LIMITED WARRANTY

Microsoft warrants that properly licensed software will perform substantially as described in any Microsoft materials that accompany the software. This limited warranty does not cover problems that you cause, that arise when you fail to follow instructions, or that are caused by events beyond Microsoft's reasonable control. The limited warranty starts when the first user acquires the software, and lasts for one year. Any supplements, updates, or replacement software that you may receive from Microsoft during that year are also covered, but only for the remainder of that one-year period or for 30 days, whichever is longer. Transferring the software will not extend the limited warranty.

Microsoft gives no other express warranties, guarantees, or conditions. Microsoft excludes all implied warranties and conditions, including those of merchantability, fitness for a particular purpose, and non-infringement. If your local law does not allow the exclusion of implied warranties, then any implied warranties, guarantees, or conditions last only during the term of the limited warranty and are limited as much as your local law allows. If your local law requires a longer limited warranty term, despite this agreement, then that longer term will apply, but you can recover only the remedies this agreement allows.

If Microsoft breaches its limited warranty, it will, at its election, either: (i) repair or replace the software at no charge, or (ii) accept return of the software (or at its election the Microsoft branded device on which the software was preinstalled) for a refund of the amount paid, if any. These are your only remedies for breach of warranty. This limited warranty gives you specific legal rights, and you may also have other rights which vary from state to state or country to country.

Except for any repair, replacement, or refund Microsoft may provide, you may not recover under this limited warranty, under any other part of this agreement, or under any theory, any damages or other remedy, including lost profits or direct, consequential, special, indirect, or incidental damages. The damage exclusions and remedy limitations in this agreement apply even if repair, replacement or a refund does not fully compensate you for any losses, if Microsoft knew or should have known about the possibility of the damages, or if the remedy fails of its essential purpose. Some states and countries

![](_page_15_Picture_1.jpeg)

do not allow the exclusion or limitation of incidental, consequential, or other damages, so those limitations or exclusions may not apply to you. If your local law allows you to recover damages from Microsoft even though this agreement does not, you cannot recover more than you paid for the software (or up to \$50 USD if you acquired the software for no charge).

Warranty Procedures

For service or a refund, you must provide a copy of your proof of purchase and comply with Microsoft's return policies, which might require you to uninstall the software and return it to Microsoft or return the software with the entire Microsoft branded device on which the software is installed; the certificate of authenticity label including the product key (if provided with your device) must remain affixed.

- 1. United States and Canada. For limited warranty service or information about how to obtain a refund for software acquired in the United States or Canada, contact Microsoft via telephone at (800) MICROSOFT; via mail at Microsoft Customer Service and Support, One Microsoft Way, Redmond, WA 98052- 6399; or visit (aka.ms/nareturns).
- 2. Europe, Middle East, and Africa. If you acquired the software in Europe, the Middle East, or Africa, Microsoft Ireland Operations Limited makes the limited warranty. To make a claim under the limited warranty, you must contact either Microsoft Ireland Operations Limited, Customer Care Centre, Atrium Building Block B, Carmanhall Road, Sandyford Industrial Estate, Dublin 18, Ireland, or the Microsoft affiliate serving your country (aka.ms/msoffices).
- 3. Australia. If you acquired the software in Australia, contact Microsoft to make a claim at 13 20 58; or Microsoft Pty Ltd, 1 Epping Road, North Ryde NSW 2113 Australia.
- 4. Other countries. If you acquired the software in another country, contact the Microsoft affiliate serving your country (aka.ms/msoffices).

EULAID: VS\_2019\_ENU.1033

#### <span id="page-15-0"></span>**4.1.3 Lua Scripting Library License Text**

Copyright (c) 1994-2018 Lua.org, PUC-Rio.

<Standard MIT License Text>

#### <span id="page-15-1"></span>**4.1.4 NumericInput Control Library License Text**

The MIT License (MIT)

Copyright (c) 2015 Johan Larsson

<Standard MIT License Text>

#### <span id="page-15-2"></span>**4.1.5 OxyPlot - Plotting Library for .NET License Text**

MIT License

Copyright (c) 2014 OxyPlot contributors

<Standard MIT License Text>

![](_page_16_Picture_1.jpeg)

#### <span id="page-16-0"></span>**4.1.6 ZLib Library License Text**

zlib.h -- interface of the 'zlib' general purpose compression library version 1.2.11, January 15th, 2017

Copyright (C) 1995-2017 Jean-loup Gailly and Mark Adler

 This software is provided 'as-is', without any express or implied warranty. In no event will the authors be held liable for any damages arising from the use of this software.

 Permission is granted to anyone to use this software for any purpose, including commercial applications, and to alter it and redistribute it freely, subject to the following restrictions:

- 1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.
- 2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.
- 3. This notice may not be removed or altered from any source distribution.

Jean-loup Gailly Mark Adler

jloup@gzip.org madler@alumni.caltech.edu

### <span id="page-16-1"></span>**4.2 Standard License Text**

#### <span id="page-16-2"></span>**4.2.1 Standard MIT License**

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.